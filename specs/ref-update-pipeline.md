# Update pipeline (reference)

Status: implemented

## Scope

Files: `src/update.rs` (the `ReleaseUpdate::update` / `update_extended` flow and the
shared helpers `choose_latest_release`, `resolve_and_confirm`, `build_download`,
`finish_update` / `finish_update_owned`, `install_binary`, `verify_signature`, plus the async
sibling `update_extended_async` and the public sealed `AsyncReleaseUpdate` trait) and
`src/lib.rs` (the `Download`, `Extract`, `ArchiveKind`, `Compression`, `Move`, and `MoveAll`
install primitives). This subsystem is the end-to-end install pipeline: how a built updater turns
"there is a newer release" into a replaced on-disk binary.

## Behavior

### Entry points

`update()` calls `update_extended()` and maps its result through
`ReleaseStatus::into_version_status(current_version)`. `update_extended()` is the sync flow; the
free `update::update_extended_async()` is the async flow, which differs in that the release listing
and the download are awaited and the verify/extract/replace tail runs on
`tokio::task::spawn_blocking`. The sync and async paths share the same selection/asset/download
helpers and the same verify/extract/replace tail (`finish_update_owned`).

The verify/extract/replace tail is `finish_update_owned(ctx, dir: TempDir, archive: &Path)`, which
takes a `FinishCtx` of **owned** fields (install path, target, bin name, in-archive path,
show_output, the verify callback, and under the features the owned checksum and verifying keys) and
the `TempDir` moved in by value. The sync `finish_update(&U, release, dir, archive)` builds the ctx
from the updater and calls the owned twin inline (no spawn). The async path builds the same ctx,
moves the `TempDir` into the closure, and runs `finish_update_owned` inside
`tokio::task::spawn_blocking(move || ...)`, awaiting the join handle and mapping a `JoinError` to
`Error::Update`. So the async update never blocks the executor on the verify/extract/replace work,
and `update_extended_async`'s future stays `Send` (the `PageRequest::parse` parser is `+ Send`).

### Fetch and select

1. Print the target-arch / current-version header (`print_check_header`, `update.rs:620`),
   gated on `show_output`.
2. If `release_tag()` is set, fetch exactly that tag via `get_release_version` (`update.rs:600`).
   Otherwise fetch the candidate list via `get_latest_releases()` and run
   `choose_latest_release` (`update.rs:595`, `631`), which: filters to releases strictly newer
   than the current version (`bump_is_greater`), sorts them semver-descending so selection is
   order-independent, prefers the newest semver-*compatible* release, else falls back to the
   newest available (flagged "*NOT* compatible"), else returns `Ok(None)` => `UpToDate`
   (`update.rs:640-711`). Unparseable versions are dropped by the leading
   `.unwrap_or(false)` filter and never reach the comparator.
3. `resolve_and_confirm` (`update.rs:716`) selects the asset: a custom `asset_matcher()` closure
   if present, otherwise `Release::asset_for(target, asset_identifier())`
   (`update.rs:86`), which matches by `target` substring (optionally `identifier`), then by
   `OS`+`ARCH` substring, then by `identifier` alone. No match => `Error::Release`
   "No asset found for target".

### Download

`resolve_and_confirm` prints the release-status block and (unless `no_confirm`) prompts
(see below). Then a `tempfile::TempDir` is created and the asset is downloaded to
`<tmpdir>/<asset.name>`. `build_download` builds the `Download` from the asset URL, applies
auth/`api_headers`, sets `ACCEPT: application/octet-stream`, merges the user's
`request_headers()` after (so a same-named user header overrides), forwards the injected HTTP
client, per-request timeout, retry budget, progress callback, progress style, and the unified
`allow_insecure_http` flag (`u.allow_insecure_http()` -> `download.allow_insecure_http(...)`).
The download is driven by `download_to` (sync) or `download_to_async` (async).

`Download` enforces a HTTPS-by-default policy: `download_to` and `download_to_async` both
reject any URL whose scheme is `http://` (case-insensitive) unless `.allow_insecure_http(true)`
was called. The check fires at call time, not at construction time. `from_url` sets
`http_allowed = false`; `.allow_insecure_http(true)` sets it to true. An `http://` URL without
the opt-in returns `Error::Config`. `build_download` forwards the unified builder flag so a
single `.allow_insecure_http(true)` on the updater builder covers both endpoint validation at
`build()` time and the artifact download; callers no longer need to call it on the `Download`
directly.

`Download::request_header` is infallible (`-> &mut Self`). If a header name or value cannot
be converted, the error is deferred: the first conversion failure is stored in
`Download::header_error` and `download_to` / `download_to_async` surface it as
`Error::InvalidHeader` before any network activity. Subsequent calls with valid headers still
succeed; subsequent calls with invalid headers are ignored (first error wins).

### Extract

`finish_update` (`update.rs:770`) runs verification (below), then extracts. The in-archive
binary path comes from `bin_path_in_archive()` with `{{ version }}`, `{{ target }}`, and
`{{ bin }}` placeholders substituted (`update.rs:792-800`). Extraction is
`Extract::from_source(archive).extract_file(tmpdir, bin_path)` (`update.rs:802`). Archive kind
is detected from the file extension by `detect_archive` (`lib.rs:588`) unless overridden via
`Extract::archive`: `.zip` => `Zip`; `.tar` => `Tar(None)`; `.tgz` and `.tar.gz` =>
`Tar(Some(Gz))`; a bare `.gz` => `Plain(Some(Gz))`; anything else => `Plain(None)`. A kind
whose feature is not enabled yields `Error::ArchiveNotEnabled` (`lib.rs:602`). `ArchiveKind`
(`lib.rs:574`) and `Compression` (`lib.rs:584`, only `Gz`) are `#[non_exhaustive]`; the `Tar`
and `Zip` variants are feature-gated on `archive-tar` / `archive-zip`. `Plain` files are
copied (gz-decoded if `compression-tar-gz`), `Tar` is unpacked via the `tar` crate, `Zip` via
the `zip` crate (`lib.rs:805-885`). The extracted binary is `<tmpdir>/<bin_path>`
(`update.rs:803`).

**ZIP path traversal (zip-slip) guard.** For every zip entry, `extract_into` and
`extract_file` call `ZipFile::enclosed_name()` before joining the entry path to `into_dir`.
`enclosed_name()` returns `None` for any entry whose name contains `..` components, is
absolute, or would otherwise escape the destination directory. A `None` result aborts
extraction with `Error::Internal` naming the offending entry. The `zip` crate's `name()` is
never joined to a filesystem path directly.

**Decompression-bomb guard.** The constant `MAX_EXTRACT_BYTES` (`lib.rs`, 512 MiB) caps total
extracted bytes. For `Plain` and `Gz` entries the reader is wrapped with
`.take(MAX_EXTRACT_BYTES + 1)` and the written byte count is checked against the limit; for
`Zip` archives `extract_into` tracks cumulative bytes written across all entries, and
`extract_file` caps the single entry. Exceeding the limit aborts with `Error::Io` (kind
`InvalidData`) citing the decompression-bomb guard. The cap applies after decompression, so
a highly compressed payload is still bounded.

### Verify ordering

In `finish_update`, before any extraction or replacement:

1. **Unverified-update warning.** `ctx_is_unverified(&ctx)` checks whether neither a checksum
   nor any signing keys are configured. If so, `log::warn!` emits a message at the `warn` level
   naming `bin_name`. This is a warn-and-continue: the update proceeds regardless.
2. **Checksum** (feature `checksums`): if `verify_checksum()` is set, `checksum.verify(archive_path)`
   on the downloaded archive; a mismatch aborts here (`update.rs:806-809`).
3. **Signature** (feature `signatures`): `verify_signature(archive_path, verify_keys())`
   (`update.rs:811`). Empty key set is a no-op; otherwise the archive is detected and verified
   with zipsign (`verify_tar` for `Tar(Some(Gz))`, `verify_zip` for `Zip`), keyed with the
   archive file name as context; any other kind => `Error::NoSignatures(kind)`
   (`update.rs:904-947`), whose message names the kind via its `Display` impl
   (`tar.gz` / `zip` / `tar` / `gz` / `plain`), e.g. "signature verification is only
   implemented for `.tar.gz` and `.zip` assets, not gz files".

Both run on the *downloaded archive bytes* and before extraction. The third hook,
`verify_with`, runs later inside `install_binary` (`update.rs:872`) on the *extracted binary*,
immediately before the swap. Ordering: warn-if-unverified -> verify_checksum -> verify_keys ->
extract -> verify_with -> replace.

### Replace

`install_binary` (`update.rs:867`): runs the `verify_with` hook first; `false` => bail
`Error::Update` "post-update verification rejected the new binary" with nothing replaced. Then
if `bin_install_path()` equals `std::env::current_exe()`, the swap goes through
`self_replace::self_replace(new_exe)` (atomic in-place replace of the running exe,
`update.rs:882`). Otherwise `Move::from_source(new_exe).to_dest(bin_install_path)`
(`update.rs:884`). `Move::to_dest` (`lib.rs:928`) renames source -> dest; with
`replace_using_temp` set and an existing dest, it first renames dest aside to the temp path
and renames it back if the source->dest rename fails (rollback). `rename` cannot cross
filesystems, so source, dest, and temp must share one. The high-level flow does not call
`replace_using_temp`.

### Multi-file install

`MoveAll` (`lib.rs:988`) is the transactional multi-file primitive, not used by the
single-binary `update()` flow; callers drive it by hand after extracting an archive
themselves. `from_temp(temp)` starts it, `add(source, dest)` queues moves, `commit()` applies
them in order (`lib.rs:1021`). Each existing destination is stashed under `temp` so it can be
restored; on the first failed rename, the just-stashed dest is restored and all
already-applied moves are rolled back in reverse via `rollback` (`lib.rs:1083`), restoring
stashed originals or removing freshly-installed files, and the original error is returned.
Rollback is best-effort: a failing rollback step is logged via `log::error!`, not surfaced.
`commit` drains the queue (`std::mem::take`), so a second `commit` is a no-op returning
`Ok(())`. All sources, destinations, and `temp` must be on one filesystem (`rename`).

### Confirm and output

`no_confirm()` controls the prompt; `show_output()` controls informational printing. In
`resolve_and_confirm` (`update.rs:724-735`), the release-status block (current exe, new exe
name, download URL, "will be downloaded/extracted and replaced") prints when either
`show_output` is true or a confirmation will be prompted, so it prints even with
`show_output(false)` unless `no_confirm(true)` is also set. The confirmation prompt
(`confirm("Do you want to continue? [Y/n] ")`, `lib.rs:521`) reads stdin; blank or `y`
continues, anything else => `Error::Aborted` (Display "AbortedError: the update was not
confirmed", `lib.rs:528`). `print_check_header`,
`finish_update`'s "Extracting archive..."/"Done"/"Replacing binary file..." messages, and
`choose_latest_release`'s release messages are all gated on `show_output`
(`print_flush`/`println` helpers, `update.rs:890-902`). `show_download_progress()` toggles the
`indicatif` terminal bar in `Download` (`lib.rs:1329`); the bar is suppressed when the server
sends no `Content-Length`. An independent `progress_callback` fires per chunk regardless of
the bar.

### Status reported

`ReleaseStatus` (`update.rs:41`) is `UpToDate` or `Updated(Release)` (carries the full installed
`Release`). `update_extended` returns `Updated(release)` after a successful install
(`update.rs:844`) or `UpToDate` when nothing newer was found. `update()` collapses this to
`VersionStatus` (`lib.rs:545`), `UpToDate(String)` / `Updated(String)` carrying only the version tag,
via `into_version_status`.

## Public surface

- `update::ReleaseUpdate` (sealed): `update(&self) -> Result<VersionStatus>`,
  `update_extended(&self) -> Result<ReleaseStatus>`, plus `get_latest_release`,
  `get_latest_releases`, `get_release_version`. Accessors live on the sealed `UpdateConfig`
  supertrait.
- `update::AsyncReleaseUpdate` (sealed via `UpdateConfig: sealed::Sealed`, feature `async`): the
  async counterpart of `ReleaseUpdate`. Fetch verbs `get_latest_release_async`,
  `get_latest_releases_async`, `get_release_version_async`, plus default-bodied `update_async` (->
  `VersionStatus`) and `update_extended_async` (-> `ReleaseStatus`) that route to the free
  `update::update_extended_async`. Its methods are RPITIT (`impl Future<Output = ...> + Send`), so
  the trait is not object-safe (nameable and usable as a generic bound, like `AsyncReleaseSource`,
  but never `dyn`). Bring it into scope to call the verbs.
- `update::ReleaseStatus` (`#[non_exhaustive]`): `into_version_status`, `is_up_to_date`, `is_updated`.
- `VersionStatus` (`#[non_exhaustive]`): `version`, `is_up_to_date`, `is_updated`, `Display`.
- `Download` (`#[must_use]`): `from_url`, `allow_insecure_http`, `show_download_progress`,
  `timeout`, `progress_callback`, `progress_style`, `replace_headers`, `request_header`
  (infallible, deferred error), `download_to`, `download_to_async` (feature `async`),
  `reqwest_client`/`reqwest_async_client`/`ureq_agent` (client-gated).
- `Extract<'a>` (`#[must_use]`): `from_source`, `archive`, `extract_into`, `extract_file`.
- `ArchiveKind` (`#[non_exhaustive]`): `Plain(Option<Compression>)`, `Tar(...)` (feature
  `archive-tar`), `Zip` (feature `archive-zip`). `Compression` (`#[non_exhaustive]`): `Gz`.
- `Move<'a>` (`#[must_use]`): `from_source`, `replace_using_temp`, `to_dest`.
- `MoveAll<'a>` (`#[must_use]`, `#[non_exhaustive]`): `from_temp`, `add`, `commit`.
- `MAX_EXTRACT_BYTES: u64` (512 MiB): the decompression-bomb cap applied to all extraction paths.

Async `update_async` / `update_extended_async` are default methods on the public sealed
`AsyncReleaseUpdate` trait, implemented by each backend's `Update` (and the custom `AsyncUpdate`)
under feature `async`; the free `update::update_extended_async` they route to is `pub(crate)`.

## Invariants and regression checklist

- Verify-before-replace: checksum and signature both run on the downloaded archive *before*
  extraction; `verify_with` runs on the extracted binary *before* the swap. Nothing is
  replaced if any of the three rejects (`update.rs:778-784`, `872-879`).
- Order independence: `choose_latest_release` sorts candidates semver-descending and filters
  to strictly-newer, so a custom source's unordered/stale list selects correctly and never
  re-installs the current version (`update.rs:640-711`).
- Download/extract happen entirely under a `tempfile::TempDir`; it is cleaned up on drop. The
  running exe is replaced atomically via `self_replace` when it is the install target.
- `MoveAll` is all-or-nothing: success replaces every dest, first failure restores every
  destination to its prior contents; the original error (not a rollback error) is returned;
  rollback failures are logged only. A second `commit` is a no-op.
- The status block prints when `show_output || !no_confirm`; the prompt prints only when
  `!no_confirm`. Suppressing one does not suppress the other.
- The download is never retried; user `request_headers` override the crate's ACCEPT/auth
  headers on the download.
- `update()` reports `VersionStatus` (version only); `update_extended()` reports `ReleaseStatus`
  (`UpToDate` or `Updated(Release)`).
- The async path never blocks the executor on the finish tail: `finish_update_owned` runs inside
  `tokio::task::spawn_blocking` over owned fields, with the `TempDir` moved into the closure. The
  sync and async paths share the same owned finish tail, so verify/extract/replace behavior is
  identical (sync/async parity). `update_extended_async`'s future is `Send` (the page parsers are
  `+ Send`).
- ZIP path traversal: any zip entry whose name contains `..` components or an absolute path
  is rejected with `Error::Internal` before any file is written. `enclosed_name()` is used for
  all path joins; the raw `name()` string is never joined to a filesystem path.
- Decompression-bomb cap: total extracted bytes across all entries (zip) or per-file (plain/gz)
  are capped at `MAX_EXTRACT_BYTES` (512 MiB). Exceeding the cap aborts with `Error::Io`
  (kind `InvalidData`) before the extracted file is fully written.
- HTTPS enforcement: `download_to` and `download_to_async` reject `http://` URLs unless
  `.allow_insecure_http(true)` was called. The check fires at call time, not at construction.
- Infallible `request_header`: a conversion failure is stored as `Download::header_error` and
  surfaced as `Error::InvalidHeader` at `download_to` time. Valid headers inserted before or
  after a bad call are not affected; only the first bad call's error is stored.
- Unverified-update warning: if neither a checksum nor signing keys are configured,
  `finish_update_owned` emits a `log::warn!` before proceeding. This is warn-and-continue; it
  does not block the install.

## Tests

`update.rs` `mod tests`: `choose_latest_release_*` (up-to-date / prefers-newest-compatible /
sorts-out-of-order / ignores-unparseable / falls-back-to-incompatible);
`install_binary_aborts_when_verify_rejects`, `install_binary_installs_when_verify_accepts`;
`finish_update_rejects_a_mismatched_checksum_before_extracting`,
`finish_update_passes_a_matching_checksum_then_proceeds` (feature-gated);
`ctx_is_unverified_true_when_nothing_configured`,
`ctx_is_unverified_false_when_checksum_configured` (checksums feature). `lib.rs` `mod tests`:
`detect_*` (archive detection), `unpack_*` / `test_extract_into` / `test_extract_file`
(extraction), `move_all_commits_every_move`, `move_all_rolls_back_on_failure`,
`move_all_installs_fresh_destinations`, `move_all_second_commit_is_a_noop`,
`download_invokes_progress_callback`, the `download_header_*` / `replace_headers_*` header
tests, `download_header_invalid_name_deferred_to_download_time` /
`download_header_invalid_value_deferred_to_download_time` (deferred-error path),
`download_rejects_http_url_by_default`, `download_allows_http_when_opted_in`,
`download_allows_https_by_default` (URL scheme enforcement),
`zip_extract_into_rejects_traversal_entry`, `zip_extract_file_rejects_traversal_stored_name`
(zip-slip regression), `zip_extract_into_rejects_oversized_content` (decompression-bomb),
and `status_is_up_to_date`. Doctests in the `lib.rs` crate docs cover the manual
download/extract/replace and `MoveAll` flows.

## Related

- `ref-signatures-and-checksums.md` (verify primitives), `checksum-verification.md`,
  `checksum-from-asset.md`
- `post-update-verify.md` (the `verify_with` hook)
- `multi-file-install.md` (`MoveAll`)
- `progress-callback.md` (download progress)
- `custom-asset-matching.md` (the `asset_matcher` override)
- `choose-latest-release-sort.md` (selection ordering)
- `async-api.md` (the async update path)
- `transport-control.md` (download client/headers/timeout)

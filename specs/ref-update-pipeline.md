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
show_output, the verify callback, and under the features the owned checksum, the selected asset's
release-published digest plus the `verify_release_digest` flag, and verifying keys) and
the `TempDir` moved in by value. The sync `finish_update(&U, release, &target_asset, dir, archive)`
builds the ctx from the updater and the selected asset and calls the owned twin inline (no spawn). The async path builds the same ctx,
moves the `TempDir` into the closure, and runs `finish_update_owned` inside
`tokio::task::spawn_blocking(move || ...)`, awaiting the join handle and mapping a `JoinError` to
`Error::Internal { message, source }`. So the async update never blocks the executor on the verify/extract/replace work,
and `update_extended_async`'s future stays `Send` (the `PageRequest::parse` parser is `+ Send`).

### Fetch and select

1. Print the target-arch / current-version header (`print_check_header`, `src/update.rs:print_check_header`),
   gated on `show_output`.
2. If `release_tag()` is set, fetch exactly that tag via `get_release_version`.
   Otherwise fetch the candidate list via `get_newer_releases()` and run
   `choose_latest_release`, which: filters to releases strictly newer
   than the current version (`bump_is_greater`), sorts them semver-descending so selection is
   order-independent, then applies the `update_strategy()`: under `UpdateStrategy::Compatible`
   (default) it prefers the newest semver-*compatible* release and falls back to the newest
   available (flagged "*NOT* compatible"); under `UpdateStrategy::Latest` it always takes the
   newest available, even across a major bump. Empty candidate list => `Ok(None)` => `UpToDate`
   (`src/update.rs:choose_latest_release`). Unparseable versions are dropped by the leading
   `.unwrap_or(false)` filter and never reach the comparator.
3. `resolve_and_confirm` (`src/update.rs:resolve_and_confirm`) selects the asset: a custom `asset_matcher()` closure
   if present, otherwise `Release::asset_for(target, asset_identifier())`
   (`src/update.rs:Release::asset_for`), which matches by `target` substring (optionally `identifier`), then by
   `OS`+`ARCH` substring, then by `identifier` alone. No match =>
   `Error::NoReleaseFound { target: Some(...) }`. A server-supplied asset name that is empty,
   `.`/`..`, contains a path separator, contains a control character, or is absolute =>
   `Error::InvalidAssetName { name }` before any file is created. The control-character rejection
   is a display defense, not a path defense: the name is echoed into the confirmation block, where
   a `\r` or an ESC sequence could repaint the lines the user reads before authorizing the
   replacement.

### Download

`resolve_and_confirm` prints the release-status block and (unless `no_confirm`) prompts
(see below). If `check_install_path_writable()` is `true`, `probe_writable` (`src/update.rs:probe_writable`) runs
immediately after the confirmation and before any download, probing the bundle's parent directory in
bundle mode (`probe_dir_writable`) and otherwise `bin_install_path`
(`probe_install_path_writable`)
(`src/update.rs:update_extended` sync, `src/update.rs:update_extended_async` async): only a definite `PermissionDenied`
errors as `Error::InstallPathNotWritable { path }`; any other result (missing parent directory,
unusual filesystem, `Ok`) proceeds. Default is `false` (off). Then a `tempfile::TempDir` is
created and the asset is downloaded to `<tmpdir>/<asset.name>` (`src/update.rs:update_extended`). `build_download` (`src/update.rs:build_download`) builds the
`Download` from the asset URL, applies auth/`api_headers`, sets `ACCEPT:
application/octet-stream`, merges the user's `request_headers()` *after* (so a same-named
user header overrides), forwards the injected HTTP client, per-request timeout, progress
callback, and progress style. The download is driven by `download_to` (sync, `src/lib.rs:download_to`)
or `download_to_async` (`src/lib.rs:download_to_async`). The retry budget covers the download's request-establishment phase (before bytes stream); mid-stream failures are not retried.

### Extract

`finish_update` (`src/update.rs:finish_update`) runs verification (below), then extracts. The in-archive
binary path comes from `bin_path_in_archive()` with `{{ version }}`, `{{ target }}`, and
`{{ bin }}` placeholders substituted (`src/update.rs:substitute`). Extraction is
`Extract::from_source(archive).extract_file(tmpdir, bin_path)` (`src/update.rs:finish_update_owned`). Archive kind
is detected from the file extension by `detect_archive` (`src/lib.rs:detect_archive`) unless overridden via
`Extract::archive`: `.zip` => `Zip`; `.tar` => `Tar(None)`; `.tgz` and `.tar.gz` =>
`Tar(Some(Gz))`; a bare `.gz` => `Plain(Some(Gz))`; `.txz` and `.tar.xz` => `Tar(Some(Xz))`;
a bare `.xz` => `Plain(Some(Xz))`; anything else => `Plain(None)`. A kind whose archive
feature is not enabled yields `Error::ArchiveNotEnabled`, and a recognized compression whose
codec feature is off (a `.gz` without `compression-tar-gz`, a `.xz` without
`compression-tar-xz`) yields `Error::CompressionNotEnabled` rather than installing the still
-compressed bytes (`src/lib.rs:detect_archive`). `ArchiveKind` (`src/lib.rs:ArchiveKind`) and `Compression` (`Gz`, `Xz`)
are `#[non_exhaustive]`; the `Tar` and `Zip` variants are feature-gated on `archive-tar` /
`archive-zip`. `Plain` files are copied (gz/xz-decoded when the matching codec feature is on),
`Tar` is unpacked via the `tar` crate over the decoded stream, `Zip` via the `zip` crate
(`src/lib.rs:extract_into`, `src/lib.rs:extract_file`). The extracted binary is `<tmpdir>/<bin_path>`
(`src/update.rs:finish_update_owned`).

Zip entry names that would escape `into_dir` are rejected via `enclosed_name` (zip-slip defense,
`src/lib.rs:enclosed_name`). On unix, `extract_into` restores a zip symlink entry (unix mode carrying
`S_IFLNK`, detected by `ZipFile::is_symlink`) as a real symlink rather than writing its target
string out as a regular file (`src/lib.rs:extract_into`), preserving symlink-dependent trees such as a macOS
`.app` bundle's `Frameworks/*/Versions/Current` links; `tar` extraction already restores symlinks
itself. A symlink target that escapes the extraction root (absolute, or `..` resolving above
`into_dir`) is rejected by `symlink_target_escapes` with an `Error::Internal`, mirroring the
entry-name defense; a duplicate entry at the link path is removed before the link is created, and
no permission bits are applied to the link. That target check is purely lexical, so it cannot catch
a symlinked intermediate directory that aliases an entry's parent to a shallower physical path (the
symlinked-parent traversal: `d/sl -> ..` then `d/sl/evil -> ../../x`, lexically in-bounds but
physically above the root). As a backstop, the extraction root is canonicalized once up front and,
for every zip entry (symlink or regular file), after its parents are materialized the physical
parent is canonicalized and must equal `canonical_root` joined with the entry's lexical parent;
descent through a symlinked ancestor (or a canonicalize failure) is rejected with an
`Error::Internal`, while descent through real directories is allowed. On non-unix targets the
symlink-restore
block is compiled out and such an entry is written as a regular file (creating symlinks needs
elevated privileges on windows). `extract_file` errors on a symlink entry (`src/lib.rs:extract_file`) rather
than writing its target string as the requested file.

### Verify ordering

In `finish_update`, before any extraction or replacement:

1. **Checksum** (feature `checksums`): if `verify_checksum()` is set, `checksum.verify(archive_path)`
   on the downloaded archive; a mismatch aborts here (`src/update.rs:finish_update_owned`).
1b. **Published sums digest** (feature `checksums`): if `checksum_from_asset()` names an asset, the
   digest resolved from it before the download (see below) is verified the same way, independently
   of gate 1 (`src/update.rs:finish_update_owned`).
2. **Release digest** (feature `checksums`): if `verify_release_digest()` is on (the default) and
   the selected asset carries a backend-published digest (`ReleaseAsset::digest()`, the
   `algorithm:hex` form github publishes per asset), the digest is parsed via
   `Checksum::parse_digest` and verified against the archive (`src/update.rs:finish_update_owned`). A digest
   that is present but malformed or an unsupported algorithm aborts with
   `Error::InvalidResponse` naming the digest (no silent skip); an absent digest skips the gate.
   Independent of gate 1: when both apply, both must pass.
3. **Signature** (feature `signatures`): `verify_signature(archive_path, verifying_keys())`
   (`src/update.rs:verify_signature`). Empty key set is a no-op; otherwise the archive is detected and verified
   with zipsign (`verify_tar` for `Tar(Some(Gz))`, `verify_zip` for `Zip`), keyed with the
   archive file name as context; any other kind => `Error::NoSignatures(kind)`,
   whose message names the kind via its `Display` impl
   (`tar.gz` / `zip` / `tar` / `gz` / `plain`), e.g. "signature verification is only
   implemented for `.tar.gz` and `.zip` assets, not gz files".

4. **Archive hook**: if `verify_archive_callback()` is set, it is called with the archive path
   (`src/update.rs:run_archive_verify_hook`, from `src/update.rs:finish_update_owned`). `Err(..)`
   aborts with `Error::ArchiveVerificationRejected { reason }`, nothing extracted and nothing
   installed; an error that already is an `ArchiveVerificationRejected` passes through unwrapped,
   any other error's message becomes the reason. This is the caller's own gate over the artifact as
   published -- an external attestation/signature check (`gh attestation verify`,
   `cosign verify-blob`) whose subject is the release file itself. It runs *after* gates 1-3 so a
   corrupt download is rejected by the cheap built-in digest check before an external tool is
   spawned on it.

The sums digest itself is resolved *before* the artifact download, in both orchestrators
(`src/update.rs:update_extended`, `src/update.rs:update_extended_async`): after the asset is selected
and confirmed, `sums_asset_for` finds the release asset named by `checksum_from_asset()`
(absent => `Error::ChecksumSourceInvalid`, never a skipped check), `build_sums_download` fetches it
over the artifact download's transport with progress reporting cleared
(`src/lib.rs:Download::clear_progress_reporting`), and `checksum_from_sums_bytes` ->
`Checksum::from_sums_file` (`src/checksum.rs`) parses the entry for the selected asset's file name.
Resolving first means a release missing the sums asset fails without pulling the artifact. The
algorithm comes from the digest length (64 -> sha256, 128 -> sha512).

All four gates run on the *downloaded archive bytes* and before extraction. The last hook,
`verify_binary`, runs later inside `install_binary` on the *extracted binary* (in bundle mode, on
the *staged bundle root*), immediately before the swap. Ordering: verify_checksum -> release
digest -> verify_keys -> verify_archive -> extract -> verify_binary -> replace.

The two hooks are deliberately distinct error variants (`ArchiveVerificationRejected` vs
`VerificationRejected`) because they see different files: a caller registering both can tell which
one refused the update.

### Replace

`install_binary` (`src/update.rs:install_binary`): runs the `verify_binary` hook first; `Err(..)` => bail
`Error::VerificationRejected { reason }` with nothing replaced. Then
if `bin_install_path()` equals `std::env::current_exe()`, the swap goes through
`self_replace::self_replace(new_exe)` (atomic in-place replace of the running exe,
`src/update.rs:install_binary`). Otherwise `Move::from_source(new_exe).to_dest(bin_install_path)`
(`src/update.rs:install_binary`). `Move::to_dest` (`src/lib.rs:Move::to_dest`) renames source -> dest; with
`replace_using_temp` set and an existing dest, it first renames dest aside to the temp path
and renames it back if the source->dest rename fails (rollback). `rename` cannot cross
filesystems, so source, dest, and temp must share one. The high-level flow does not call
`replace_using_temp`.

Both the `self_replace` call and the `Move::to_dest` call have their IO errors wrapped by
`map_install_io_error` (`src/update.rs:map_install_io_error`): a `PermissionDenied` becomes
`Error::InstallPathNotWritable { path }` naming the install path; any other `io::Error` kind is
rewrapped as `Error::Io` with the message `"installing to {path}: {orig}"`, preserving the
original `ErrorKind` for inspection. This annotation is always on, independent of the opt-in
preflight probe (`check_install_path_writable`).

### Bundle install (directory bundles)

`bundle_path_in_archive()` being `Some` selects bundle mode, resolved at `build()` time by
`CommonBuilderConfig::resolve_bundle_mode` (`src/backends/common.rs:CommonBuilderConfig::resolve_bundle_mode`): an explicit
`bin_install_path` or a non-auto `bin_path_in_archive` alongside it is
`Error::ConflictingConfig { field, conflict }`; a `bundle_install_path` set *without*
`bundle_path_in_archive` is `Error::MissingField { field: "bundle_path_in_archive" }` rather than a
silently discarded path; and an unset `bundle_install_path` resolves via
`default_bundle_install_path` (`src/update.rs:default_bundle_install_path`) -- on macOS the nearest `.app` ancestor of
`current_exe()` (`enclosing_app_bundle`, `src/update.rs:enclosing_app_bundle`), with a translocated exe
(`is_translocated`, `src/update.rs:is_translocated`) rejected as `Error::AppTranslocated` and no `.app` ancestor as
`Error::NoAppBundle`; on every other target `Error::MissingField { field: "bundle_install_path" }`.

In the finish tail the same `{{ bin }}` / `{{ target }}` / `{{ version }}` substitution runs over
the bundle path, then `install_bundle` (`src/update.rs:install_bundle`) replaces the single-file
extract-and-install pair: the configured path is first run through `resolve_bundle_target`, which
maps a live symlink to the tree it designates (`rename` does not follow a path's final component, so
swapping onto the link itself would stash the link and orphan the installed tree; a dangling link and
a plain path pass through unchanged); two `tempfile::TempDir`s (staging and stash) are created inside
`install_parent(<resolved target>)`, so every rename is same-filesystem and there is no cross-device
case; `Extract::extract_into` unpacks the whole archive into staging; the staged root is
`staging/<substituted bundle path>`. Failure to create either temp dir goes through
`map_install_io_error` naming the bundle path.

`swap_bundle` (`src/update.rs:swap_bundle`) performs the swap, taking the running exe as a parameter (so it is
testable against a temp tree). Pre-swap checks, none of which touch the destination: the staged root
must exist and be a directory (else `Error::Io` NotFound naming it); when `exe_inside_bundle`
(`src/update.rs:exe_inside_bundle`, canonicalizing both sides like `same_file`) reports the running exe inside the
installed bundle, the staged tree must carry a file at the same relative path; then the
`verify_binary` hook runs against the *staged bundle root* via the shared `run_verify_hook`
(`src/update.rs:run_verify_hook`). Then, in order: rename the running exe to `stash/exe-aside` (only when it is
inside the bundle), rename `bundle_install_path` to `stash/old` (only when it exists), rename the
staged root onto `bundle_install_path`. A failure at either later step reverses the applied renames
(old tree first, then the exe, via `restore_stashed`, `src/update.rs:restore_stashed`) and returns the original
error mapped by `map_install_io_error`; rollback is best-effort and logged, matching the `MoveAll`
contract. After the final rename the update is committed and the file at the running exe's path is
the new tree's executable, so no `self_replace` call is involved. On unix the stashed old image is
unlinked with the stash `TempDir`; on windows it may stay locked until process exit, which never
affects the installed tree. The swap is one code path on all targets: a windows bundle holding other
open files (a loaded DLL) fails at the directory rename and rolls back.

Output messages in bundle mode are "Extracting archive... Done" then "Replacing bundle directory...
Done"; the confirmation block names the bundle path ("Current bundle:") and says the existing bundle
directory will be replaced, since `bin_install_path` is never written in bundle mode.
`ReleaseStatus` / `VersionStatus` reporting is unchanged.

Existence at the destination is tested with `fs::symlink_metadata`, not `exists()`: a dangling
symlink is an entry that must be stashed out of the way (renaming a directory onto one fails with
`ENOTDIR`), where `exists()` would report it absent. Concurrency is not coordinated: the existence
test and the renames are not atomic as a unit, so racing updaters can interleave.

### Multi-file install

`MoveAll` (`src/lib.rs:MoveAll`) is the transactional multi-file primitive, not used by the
single-binary `update()` flow; callers drive it by hand after extracting an archive
themselves. `from_temp(temp)` starts it, `add(source, dest)` queues moves, `commit()` applies
them in order (`src/lib.rs:commit`). Each existing destination is stashed under `temp` so it can be
restored; on the first failed rename, the just-stashed dest is restored and all
already-applied moves are rolled back in reverse via `rollback` (`src/lib.rs:rollback`), restoring
stashed originals or removing freshly-installed files, and the original error is returned.
Rollback is best-effort: a failing rollback step is logged via `log::error!`, not surfaced.
`commit` drains the queue (`std::mem::take`), so a second `commit` is a no-op returning
`Ok(())`. All sources, destinations, and `temp` must be on one filesystem (`rename`).

### Confirm and output

`no_confirm()` controls the prompt; `show_output()` controls informational printing. In
`resolve_and_confirm` (`src/update.rs:resolve_and_confirm`), the release-status block (current exe, new exe
name, download URL, "will be downloaded/extracted and replaced") prints when either
`show_output` is true or a confirmation will be prompted, so it prints even with
`show_output(false)` unless `no_confirm(true)` is also set. The install-target line is built by
`install_target_line` (`src/update.rs:install_target_line`), which formats the path with `Path::display()`, so it
prints unquoted and with one separator per component on windows; the asset name and the redacted
download URL are strings formatted with `{:?}` and stay quoted. The confirmation prompt
(`confirm("Do you want to continue? [Y/n] ")`, `src/lib.rs:confirm`) reads stdin; blank or `y`
continues, anything else => `Error::Aborted` (Display "AbortedError: the update was not
confirmed", `src/lib.rs:confirm`). `print_check_header`,
`finish_update`'s "Extracting archive..."/"Done"/"Replacing binary file..." messages, and
`choose_latest_release`'s release messages are all gated on `show_output`
(`print_flush`/`println` helpers, `src/update.rs:print_flush`, `src/update.rs:println`). `show_download_progress()` toggles the
`indicatif` terminal bar in `Download` (`src/lib.rs:show_download_progress`); the bar is suppressed when the server
sends no `Content-Length`. An independent `progress_callback` fires per chunk regardless of
the bar.

### Status reported

`ReleaseStatus` (`src/update.rs:ReleaseStatus`) is `UpToDate` or `Updated(Release)` (carries the full installed
`Release`). `update_extended` returns `Updated(release)` after a successful install
(`src/update.rs:update_extended`) or `UpToDate` when nothing newer was found. `update()` collapses this to
`VersionStatus` (`src/lib.rs:VersionStatus`), `UpToDate(String)` / `Updated(String)` carrying only the version tag,
via `into_version_status`.

## Public surface

- `update::ReleaseUpdate` (sealed): `update(&self) -> Result<VersionStatus>`,
  `update_extended(&self) -> Result<ReleaseStatus>`, plus `get_latest_release`,
  `get_newer_releases`, `get_release_version`. Accessors live on the sealed `UpdateConfig`
  supertrait. Each backend `build()` returns the concrete `Update` (`Send`), which
  exposes these verbs plus `is_update_available` as inherent methods.
- `update::AsyncReleaseUpdate` (sealed via `UpdateConfig: sealed::Sealed`, feature `async`): the
  async counterpart of `ReleaseUpdate`. Fetch verbs `get_latest_release_async`,
  `get_newer_releases_async`, `get_release_version_async`, plus default-bodied `update_async` (->
  `VersionStatus`) and `update_extended_async` (-> `ReleaseStatus`) that route to the free
  `update::update_extended_async`. Its methods are RPITIT (`impl Future<Output = ...> + Send`), so
  the trait is not object-safe (nameable and usable as a generic bound, like `AsyncReleaseSource`,
  but never `dyn`). Bring it into scope to call the verbs.
- `update::ReleaseStatus` (`#[non_exhaustive]`): `into_version_status`, `is_up_to_date`, `is_updated`.
- `VersionStatus` (`#[non_exhaustive]`): `version`, `is_up_to_date`, `is_updated`, `Display`.
- `Download`: `from_url`, `show_download_progress`, `timeout`, `progress_callback`,
  `progress_style`, `replace_headers`, `request_header`, `download_to`, `download_to_async`
  (feature `async`).
- `Extract`: `from_source`, `archive`, `extract_into`, `extract_file`; the path
  arguments take `impl AsRef<Path>` (as do `Move` / `MoveAll`), with no lifetime
  parameter on the types.
- `ArchiveKind` (`#[non_exhaustive]`): `Plain(Option<Compression>)`, `Tar(...)` (feature
  `archive-tar`), `Zip` (feature `archive-zip`). `Compression` (`#[non_exhaustive]`): `Gz`
  (feature `compression-tar-gz`), `Xz` (feature `compression-tar-xz`).
- `Move`: `from_source`, `replace_using_temp`, `to_dest`.
- `MoveAll` (`#[must_use]`, `#[non_exhaustive]`): `from_temp`, `add`, `commit`.

Async `update_async` / `update_extended_async` are default methods on the public sealed
`AsyncReleaseUpdate` trait, implemented by each backend's `Update` (and the custom `AsyncUpdate`)
under feature `async`; the free `update::update_extended_async` they route to is `pub(crate)`.

## Invariants and regression checklist

- Verify-before-replace: checksum, release digest, signature, and the `verify_archive` hook all run
  on the downloaded archive *before* extraction; `verify_binary` runs on the extracted binary
  *before* the swap. Nothing is extracted or replaced if any of the five rejects
  (`src/update.rs:finish_update_owned`, `src/update.rs:install_binary`).
- The `verify_archive` hook runs last among the archive gates, so a corrupt download is rejected by
  the built-in digest checks before the caller's external verifier is invoked
  (`src/update.rs:finish_update_owned`). It fires in bundle mode too: the hook sits ahead of the
  branch into `install_bundle`.
- `checksum_from_asset` never degrades to "no verification": a missing sums asset, a missing entry,
  a non-UTF-8 body, or an unusable digest length all abort with `Error::ChecksumSourceInvalid`
  (`src/update.rs:sums_asset_for`, `src/checksum.rs:from_sums_file`).
- The release-digest gate is on by default under `checksums` and only fires when the selected
  asset carries a digest; `verify_release_digest(false)` opts out. A present-but-unparseable
  digest is a hard `Error::InvalidResponse`, not a silent skip (`src/update.rs:finish_update_owned`).
- Order independence: `choose_latest_release` sorts candidates semver-descending and filters
  to strictly-newer, so a custom source's unordered/stale list selects correctly and never
  re-installs the current version (`src/update.rs:choose_latest_release`).
- Download/extract happen entirely under a `tempfile::TempDir`; it is cleaned up on drop. The
  running exe is replaced atomically via `self_replace` when it is the install target.
- `MoveAll` is all-or-nothing: success replaces every dest, first failure restores every
  destination to its prior contents; the original error (not a rollback error) is returned;
  rollback failures are logged only. A second `commit` is a no-op.
- The status block prints when `show_output || !no_confirm`; the prompt prints only when
  `!no_confirm`. Suppressing one does not suppress the other.
- The status block's install-target line goes through `Path::display()`, never `{:?}`: a path is
  shown exactly as the platform writes it, so a windows path keeps single backslashes
  (`install_target_line`, `src/update.rs:install_target_line`).
- The retry budget covers the download's request-establishment phase (before bytes stream); mid-stream failures are not retried. User `request_headers` override the crate's ACCEPT/auth
  headers on the download.
- When `check_install_path_writable` is `true`, the preflight probe (`probe_writable`,
  `src/update.rs:probe_writable`) runs after confirmation and before any download, targeting the bundle's parent
  directory in bundle mode and `bin_install_path` otherwise; only a definite `PermissionDenied`
  errors, indeterminate results proceed. Default is `false`.
- Bundle mode is all-or-nothing at whole-tree granularity: nothing under `bundle_install_path`
  changes until the old tree is stashed, a failure at any step restores the old tree (and the
  running exe inside it), and the original error is returned with rollback failures logged only. It
  never falls back to a copy, so an install is never partially visible; and it never calls
  `self_replace` (the exe rides along inside the swapped tree).
- Bundle mode and the single-file `bin_*` paths are mutually exclusive: setting both explicitly is
  `Error::ConflictingConfig` from `build()`, not a silently-dropped setter.
- The install step always annotates IO failures with the install path: `PermissionDenied` becomes
  `Error::InstallPathNotWritable { path }` and other kinds become `Error::Io` with the path in the
  message, `ErrorKind` preserved (`map_install_io_error`, `src/update.rs:map_install_io_error`). Independent of the
  preflight probe.
- `update()` reports `VersionStatus` (version only); `update_extended()` reports `ReleaseStatus`
  (`UpToDate` or `Updated(Release)`).
- The async path never blocks the executor on the finish tail: `finish_update_owned` runs inside
  `tokio::task::spawn_blocking` over owned fields, with the `TempDir` moved into the closure. The
  sync and async paths share the same owned finish tail, so verify/extract/replace behavior is
  identical (sync/async parity). `update_extended_async`'s future is `Send` (the page parsers are
  `+ Send`).

## Tests

`update.rs` `mod tests`: `choose_latest_release_*` (up-to-date / prefers-newest-compatible /
sorts-out-of-order / ignores-unparseable / falls-back-to-incompatible);
`install_binary_aborts_when_verify_rejects`, `install_binary_installs_when_verify_accepts`,
`install_target_line_prints_paths_without_debug_escaping`;
`finish_update_rejects_a_mismatched_checksum_before_extracting`,
`finish_update_passes_a_matching_checksum_then_proceeds`,
`finish_update_rejects_a_mismatched_release_digest_by_default`,
`finish_update_passes_a_matching_release_digest_then_proceeds`,
`finish_update_release_digest_opt_out_skips_the_gate`,
`finish_update_rejects_an_unsupported_release_digest` (feature-gated); the bundle set
`swap_bundle_installs_when_nothing_is_there`, `swap_bundle_replaces_the_whole_tree`,
`swap_bundle_rejects_a_missing_or_non_directory_staged_root`,
`swap_bundle_rolls_back_when_the_install_rename_fails`,
`swap_bundle_moves_the_running_exe_aside_and_restores_its_path`,
`swap_bundle_rollback_restores_the_running_exe_inside_the_old_tree`,
`swap_bundle_requires_the_staged_tree_to_carry_the_running_exe_path`,
`swap_bundle_verifies_the_staged_root_and_a_rejection_replaces_nothing`,
`install_bundle_extracts_and_swaps_a_real_archive` (zip fixture with an exec bit and a symlink),
`exe_inside_bundle_detects_containment_through_symlinks`,
`enclosing_app_bundle_finds_the_nearest_app_ancestor`,
`is_translocated_matches_the_translocation_mount`,
`probe_writable_probes_the_bundle_parent_in_bundle_mode`, and
`probe_writable_falls_back_to_the_bin_path_without_bundle_mode`; `backends/common.rs` `mod tests`
covers the bundle-mode resolution (`build_resolves_bundle_mode_with_an_explicit_install_path`,
`build_leaves_bundle_fields_none_without_the_setter`,
`build_rejects_bundle_mode_with_an_explicit_bin_install_path`,
`build_rejects_bundle_mode_only_with_an_explicit_bin_path_in_archive`,
`build_requires_bundle_install_path_off_macos`). `lib.rs` `mod tests`:
`detect_*` (archive detection), `unpack_*` / `test_extract_into` / `test_extract_file`
(extraction), `move_all_commits_every_move`, `move_all_rolls_back_on_failure`,
`move_all_installs_fresh_destinations`, `move_all_second_commit_is_a_noop`,
`download_invokes_progress_callback`, the `download_header_*` / `replace_headers_*` header
tests, and `status_is_up_to_date`. Doctests in the `lib.rs` crate docs cover the manual
download/extract/replace and `MoveAll` flows.

## Related

- `ref-signatures-and-checksums.md` (verify primitives), `checksum-verification.md`,
  `checksum-from-asset.md`
- `post-update-verify.md` (the `verify_binary` hook)
- `archive-verify.md` (the `verify_archive` hook)
- `multi-file-install.md` (`MoveAll`)
- `progress-callback.md` (download progress)
- `custom-asset-matching.md` (the `asset_matcher` override)
- `choose-latest-release-sort.md` (selection ordering)
- `async-api.md` (the async update path)
- `transport-control.md` (download client/headers/timeout)

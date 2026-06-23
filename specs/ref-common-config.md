# Common config and builder macros (reference)

Status: implemented

## Scope

Documents the configuration surface every backend (`github`, `gitlab`, `gitea`,
`s3`, `custom`) shares: the unvalidated builder state `CommonBuilderConfig`, its
validated form `CommonConfig`, and the three macros that emit the shared
setters, accessors, and async verbs so that surface lives in exactly one place.

Source: `src/backends/common.rs`, `src/macros.rs`. One backend
(`src/backends/github.rs`) is referenced for how `common` is embedded.

## Behavior

### CommonBuilderConfig / CommonConfig + validation

`CommonBuilderConfig` (`common.rs:85`) is the pre-validation state held while a
backend's `UpdateBuilder` is configured. Each backend's builder embeds it as a
`common: CommonBuilderConfig` field (`github.rs:193`). Its fields
(`common.rs:86-111`): `request: RequestConfig`, `target`, `asset_identifier`,
`bin_name`, `bin_install_path`, `bin_path_in_archive`, `bin_path_in_archive_auto`
(`common.rs:95`, internal `bool` tracking whether `bin_path_in_archive` was
auto-derived from `bin_name`), `show_download_progress`, `show_output`,
`no_confirm`, `current_version`, `release_tag`, `progress_template`,
`progress_chars`, `auth_token`, `progress_callback`, `verify`, `asset_matcher`,
`checksum` (under `checksums`), and `verifying_keys` (under `signatures`).

`Default` (`common.rs:113-140`) sets the non-`None` defaults:
`bin_path_in_archive_auto = false`, `show_download_progress = false`,
`show_output = true`, `no_confirm = false`,
`progress_template = DEFAULT_PROGRESS_TEMPLATE`,
`progress_chars = DEFAULT_PROGRESS_CHARS`, and `verifying_keys = vec![]`.

`build()` (`common.rs:142-190`) validates and resolves into `CommonConfig`
(`common.rs:194-216`):

- First calls `self.request.check()` (`common.rs:150`), surfacing any deferred
  `request_header` conversion failure as `Error::Config`.
- Required (each missing field yields an `Error::Config` whose message names the
  setter to call): `current_version` (`common.rs:161`, ``"`current_version`
  required (call `.current_version(...)`)"``), `bin_name` (`common.rs:166`,
  ``"`bin_name` required (call `.bin_name(...)`)"``), `bin_path_in_archive`
  (`common.rs:174`, ``"`bin_path_in_archive` required (call `.bin_name(...)` or
  `.bin_path_in_archive(...)`)"``). The last is normally set automatically by the
  `bin_name` setter, so callers only need set `bin_name`.
- Defaulted: `target` falls back to `get_target()` (`common.rs:148-151`);
  `bin_install_path` falls back to `std::env::current_exe()` (`common.rs:162-165`),
  which can itself error and propagates via `?`.
- All other fields are cloned through unchanged. Note `target` and
  `current_version` become owned `String`, and `bin_install_path` an owned
  `PathBuf`, in `CommonConfig`.

`RequestConfig` (`common.rs:29-40`) carries `timeout`, `headers`, `retries`,
`client` (override), and `header_error`. `insert_header` (`common.rs:46-72`)
stays infallible, recording the first bad name/value in `header_error`;
`check` (`common.rs:75-80`) replays it as `Error::Config`.

### Shared setter macro: impl_common_builder_setters!

`impl_common_builder_setters!` (`macros.rs:210-463`) is invoked once inside each
backend's `impl UpdateBuilder` block (`github.rs:223`) and emits every shared
setter, each writing through `self.common.*` and returning `&mut Self`. Adding a
shared setter happens here once and reaches all backends.

Two invocation forms: `()` (`macros.rs:212-224`) emits the `@shared` set plus
`auth_token` (`macros.rs:220`); `(no_auth_token)` (`macros.rs:228-230`) emits
only `@shared`, for backends like s3 that authenticate differently.

The `@shared` vocabulary (`macros.rs:231-462`):

- `current_version(impl Into<String>)` (`macros.rs:235`) - required.
- `release_tag(impl Into<String>)` (`macros.rs:247`) - used verbatim.
- `target(impl Into<String>)` (`macros.rs:255`).
- `asset_identifier(impl Into<String>)` (`macros.rs:263`).
- `bin_name(impl Into<String>)` (`macros.rs:279`) - required; appends `EXE_SUFFIX` if absent
  and (re-)derives `bin_path_in_archive` when that path is unset or was previously
  auto-derived, setting `bin_path_in_archive_auto = true`. Re-calling `bin_name` thus
  re-derives the archive path rather than leaving a stale one; an explicitly set
  `bin_path_in_archive` is sticky and is never overwritten.
- `bin_install_path<A: AsRef<Path>>(A)` (`macros.rs:300`).
- `bin_path_in_archive(impl Into<String>)` (`macros.rs:328`) - supports `{{ bin }}`,
  `{{ target }}`, `{{ version }}` substitutions; sets `bin_path_in_archive_auto = false`
  so a later `bin_name` call will not overwrite it.
- `show_download_progress(bool)` (`macros.rs:336`).
- `progress_style(impl Into<String>, impl Into<String>)` (`macros.rs:342`) -
  sets template and chars.
- `show_output(bool)` (`macros.rs:358`).
- `no_confirm(bool)` (`macros.rs:370`).
- `unattended()` (`macros.rs:378`) - one-call CI/daemon configuration: sets
  `no_confirm(true)` + `show_output(false)`. Without it the default
  (`no_confirm == false`) blocks on stdin waiting for confirmation.
- `request_config_setters!(common.request)` (`macros.rs:384`) - splices in
  `timeout`, `request_header`, `retries`, and the feature-gated client
  overrides `reqwest_client`, `reqwest_async_client`, `ureq_agent`
  (`macros.rs:14-88`).
- `progress_callback(impl Fn(u64, Option<u64>) ...)` (`macros.rs:391`).
- `asset_matcher(impl Fn(&[ReleaseAsset]) -> Option<ReleaseAsset> ...)`
  (`macros.rs:405`).
- `verify_with(impl Fn(&Path) -> bool ...)` (`macros.rs:427`) - the post-update
  hook on the extracted binary; its doc records the full verification order
  (`verify_checksum` -> signature/`verify_keys` -> extract -> `verify_with` -> replace),
  so it runs last.
- `verify_checksum(Checksum)` (`macros.rs:439`, under `checksums`).
- `verify_keys(impl Into<Vec<VerifyingKey>>)` (`macros.rs:455`, under
  `signatures`) - **replaces** the key set on each call (last call wins, unlike
  `request_header` which appends); an empty set (or never calling it) leaves
  signature verification disabled, which is not an error.
- `auth_token(impl Into<String>)` (`macros.rs:220`, only the `()` form).

### Accessor macro: impl_update_config_accessors!

`impl_update_config_accessors!` (`macros.rs:108-200`) emits a full
`impl crate::update::UpdateConfig for $t` block (`github.rs:361`) reading through
`self.common.*`. Bodies borrow, never own: `&str` for `current_version`,
`target`, `bin_name`, `bin_path_in_archive`, `progress_template`,
`progress_chars` (`macros.rs:126-161`); `Option<&str>` via `.as_deref()` for
`release_tag`, `asset_identifier`, `auth_token` (`macros.rs:132,135,162`);
`&Path` for `bin_install_path` (`macros.rs:141`); plain `bool`/`Copy` returns
for the toggles. The `#[doc(hidden)]` accessors (`macros.rs:165-193`) expose
`request_timeout`, `request_headers`, `request_client`, `progress_callback`,
`verify_callback`, `asset_matcher`, and `verify_checksum` (`macros.rs:191`,
`checksums`-gated, both `#[doc(hidden)]` and `#[cfg(feature = "checksums")]`).
`verify_keys` (`macros.rs:194-197`) is **not** `#[doc(hidden)]`: it carries only
`#[cfg(feature = "signatures")]` and returns `&[VerifyingKey]`, so it is a
documented public accessor unlike the hidden `verify_checksum` next to it.

Three invocation forms: bare `($t)` (`macros.rs:109`) for the default
`api_headers`; `($t, { ... })` (`macros.rs:112`) splices a custom `api_headers`
override into the same `impl` (github/gitlab/gitea); and `($t, where ( ... ))`
(`macros.rs:116`) for the generic custom `AsyncUpdate<S>`.

### Async verbs

The async verbs are methods on the public sealed `AsyncReleaseUpdate` trait (in `update.rs`),
implemented by each backend's `Update` (and the custom `AsyncUpdate`) under `#[cfg(feature =
"async")]`. There is no async-methods macro: each backend writes a small `impl AsyncReleaseUpdate`
with the three fetch verbs, and `update_async` / `update_extended_async` are trait default methods.
The five verbs mirror the blocking API:

- `update_async()` - delegates to `update_extended_async` then `into_version_status` (default).
- `update_extended_async()` - calls the free `update::update_extended_async(self)` (default).
- `get_latest_release_async()` - single newest release.
- `get_latest_releases_async()` - candidate releases.
- `get_release_version_async(ver: &str)` - release by tag.

## Public surface

The builder setters and the five async verbs are `pub` and reach users (the async verbs via the
public sealed `AsyncReleaseUpdate` trait, which callers bring into scope). `CommonBuilderConfig`,
`CommonConfig`, and `RequestConfig` are `pub(crate)`; the `UpdateConfig` accessor methods are
largely `#[doc(hidden)]` plumbing.

## Invariants and regression checklist

- Shared setters are defined once in `impl_common_builder_setters!`; a new
  shared setter is added there and reaches every backend builder.
- Accessors borrow through `self.common`, never own: `&str` / `Option<&str>` /
  `&Path`, no clones.
- The five async verbs stay at parity with their blocking siblings; they live on the public sealed
  `AsyncReleaseUpdate` trait (the fetch verbs implemented per backend, `update_async` /
  `update_extended_async` as defaults).
- `bin_name` (re-)derives `bin_path_in_archive` when that path is unset or was
  auto-derived (tracked by `bin_path_in_archive_auto`); an explicitly set
  `bin_path_in_archive` is sticky and survives later `bin_name` calls.
- `unattended()` sets `no_confirm(true)` + `show_output(false)` in one call; the
  default (`no_confirm == false`) blocks on stdin.
- `build()` rejects a missing `current_version`, `bin_name`, or
  `bin_path_in_archive` with an `Error::Config` whose message names the setter to
  call, and replays any deferred `request_header` error before resolving defaults.
- `target` defaults to `get_target()`, `bin_install_path` to
  `current_exe()`; `show_output` defaults `true`, the other toggles `false`.
- The `()` form emits `auth_token`; `(no_auth_token)` omits it.

## Tests

`src/backends/common.rs` unit tests (`common.rs:218-380`):
`build_requires_current_version_bin_name_and_archive_path` (`common.rs:299`),
`build_resolves_target_and_install_path_defaults` (`common.rs:323`),
`build_error_message_names_the_setter_for_current_version` (`common.rs:347`) and
`build_error_message_names_the_setter_for_bin_name` (`common.rs:362`) asserting
the required-field errors name the setter to call, and the `insert_header` /
`check` cases (`common.rs:222-296`) covering deferred invalid-name /
invalid-value errors, first-error-wins, and the ok path.

## Related

- `1.0-api-surface.md`
- `ref-release-model.md`
- `async-api.md`

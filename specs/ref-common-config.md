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
(`common.rs:86-107`): `request: RequestConfig`, `target`, `asset_identifier`,
`bin_name`, `bin_install_path`, `bin_path_in_archive`, `show_download_progress`,
`show_output`, `no_confirm`, `current_version`, `release_tag`,
`progress_template`, `progress_chars`, `auth_token`, `progress_callback`,
`verify`, `asset_matcher`, `checksum` (under `checksums`), and `verifying_keys`
(under `signatures`).

`Default` (`common.rs:109-135`) sets the non-`None` defaults:
`show_download_progress = false`, `show_output = true`, `no_confirm = false`,
`progress_template = DEFAULT_PROGRESS_TEMPLATE`,
`progress_chars = DEFAULT_PROGRESS_CHARS`, and `verifying_keys = vec![]`.

`build()` (`common.rs:143-185`) validates and resolves into `CommonConfig`
(`common.rs:189-211`):

- First calls `self.request.check()` (`common.rs:145`), surfacing any deferred
  `request_header` conversion failure as `Error::Config`.
- Required (each missing field yields `Error::Config("... required")`):
  `current_version` (`common.rs:153-156`), `bin_name` (`common.rs:158-161`),
  `bin_path_in_archive` (`common.rs:166-169`). The last is normally set
  automatically by the `bin_name` setter, so callers only need set `bin_name`.
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

`impl_common_builder_setters!` (`macros.rs:210-434`) is invoked once inside each
backend's `impl UpdateBuilder` block (`github.rs:223`) and emits every shared
setter, each writing through `self.common.*` and returning `&mut Self`. Adding a
shared setter happens here once and reaches all backends.

Two invocation forms: `()` (`macros.rs:212-224`) emits the `@shared` set plus
`auth_token` (`macros.rs:220`); `(no_auth_token)` (`macros.rs:228-230`) emits
only `@shared`, for backends like s3 that authenticate differently.

The `@shared` vocabulary (`macros.rs:231-433`):

- `current_version(impl Into<String>)` (`macros.rs:235`) - required.
- `release_tag(impl Into<String>)` (`macros.rs:249`) - aliased `target_version_tag` /
  `target_version`; used verbatim.
- `target(impl Into<String>)` (`macros.rs:257`).
- `asset_identifier(impl Into<String>)` (`macros.rs:266`) - aliased `identifier`.
- `bin_name(impl Into<String>)` (`macros.rs:281`) - required; appends `EXE_SUFFIX` if absent
  and seeds `bin_path_in_archive` only if it is still unset.
- `bin_install_path<A: AsRef<Path>>(A)` (`macros.rs:297`).
- `bin_path_in_archive(impl Into<String>)` (`macros.rs:321`) - supports `{{ bin }}`,
  `{{ target }}`, `{{ version }}` substitutions.
- `show_download_progress(bool)` (`macros.rs:326`).
- `progress_style(impl Into<String>, impl Into<String>)` (`macros.rs:333`) -
  aliased `set_progress_style`; sets template and chars.
- `show_output(bool)` (`macros.rs:349`).
- `no_confirm(bool)` (`macros.rs:361`).
- `request_config_setters!(common.request)` (`macros.rs:366`) - splices in
  `timeout`, `request_header`, `retries`, and the feature-gated client
  overrides `reqwest_client`, `reqwest_async_client`, `ureq_agent`
  (`macros.rs:14-88`).
- `progress_callback(impl Fn(u64, Option<u64>) ...)` (`macros.rs:374`) - aliased
  `set_progress_callback`.
- `asset_matcher(impl Fn(&[ReleaseAsset]) -> Option<ReleaseAsset> ...)`
  (`macros.rs:388`).
- `verify_with(impl Fn(&Path) -> bool ...)` (`macros.rs:410`) - the post-update
  hook on the extracted binary; its doc records the full verification order
  (`checksum` -> signature/`verifying_keys` -> extract -> `verify_with` -> replace),
  so it runs last.
- `checksum(Checksum)` (`macros.rs:423`, under `checksums`) - aliased
  `verifying_checksum`.
- `verifying_keys(impl Into<Vec<VerifyingKey>>)` (`macros.rs:439`, under
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
`verify_callback`, `asset_matcher`, and `checksum` (`checksums`-gated, both
`#[doc(hidden)]` and `#[cfg(feature = "checksums")]`). `verifying_keys`
(`macros.rs:194-197`) is **not** `#[doc(hidden)]`: it carries only
`#[cfg(feature = "signatures")]` and returns `&[VerifyingKey]`, so it is a
documented public accessor unlike the hidden `checksum` next to it.

Three invocation forms: bare `($t)` (`macros.rs:109`) for the default
`api_headers`; `($t, { ... })` (`macros.rs:112`) splices a custom `api_headers`
override into the same `impl` (github/gitlab/gitea); and `($t, where ( ... ))`
(`macros.rs:116`) for the generic custom `AsyncUpdate<S>`.

### Async-methods macro: impl_async_update_methods!

`impl_async_update_methods!` (`macros.rs:439-493`, `#[cfg(feature = "async")]`)
is invoked inside each backend's async `impl Update` block (`github.rs:262`) and
emits exactly five inherent async verbs that mirror the blocking API:

- `update_async()` (`macros.rs:448`) - delegates to `update_extended_async` then
  `into_status`.
- `update_extended_async()` (`macros.rs:457`) - calls
  `update::update_extended_async(self)`.
- `get_latest_release_async()` (`macros.rs:467`) - single newest release.
- `get_latest_releases_async()` (`macros.rs:477`) - candidate releases.
- `get_release_version_async(ver: &str)` (`macros.rs:486`) - release by tag.

## Public surface

The builder setters and the five async verbs are `pub` and reach users.
`CommonBuilderConfig`, `CommonConfig`, and `RequestConfig` are `pub(crate)`;
the `UpdateConfig` accessor methods are largely `#[doc(hidden)]` plumbing.

## Invariants and regression checklist

- Shared setters are defined once in `impl_common_builder_setters!`; a new
  shared setter is added there and reaches every backend builder.
- Accessors borrow through `self.common`, never own: `&str` / `Option<&str>` /
  `&Path`, no clones.
- The five async verbs stay at parity with their blocking siblings; adding or
  renaming one happens in `impl_async_update_methods!`.
- `build()` rejects a missing `current_version`, `bin_name`, or
  `bin_path_in_archive` with `Error::Config`, and replays any deferred
  `request_header` error before resolving defaults.
- `target` defaults to `get_target()`, `bin_install_path` to
  `current_exe()`; `show_output` defaults `true`, the other toggles `false`.
- The `()` form emits `auth_token`; `(no_auth_token)` omits it.

## Tests

`src/backends/common.rs` unit tests (`common.rs:213-338`):
`build_requires_current_version_bin_name_and_archive_path` (`common.rs:294`),
`build_resolves_target_and_install_path_defaults` (`common.rs:318`), and the
`insert_header` / `check` cases (`common.rs:217-291`) covering deferred
invalid-name / invalid-value errors, first-error-wins, and the ok path.

## Related

- `1.0-api-surface.md`
- `ref-release-model.md`
- `async-api.md`

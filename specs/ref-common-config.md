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
`checksum` and `verify_release_digest` (under `checksums`), and `verifying_keys`
(under `signatures`).

`Default` (`common.rs:113-140`) sets the non-`None` defaults:
`bin_path_in_archive_auto = false`, `show_download_progress = false`,
`show_output = true`, `no_confirm = false`,
`progress_template = DEFAULT_PROGRESS_TEMPLATE`,
`progress_chars = DEFAULT_PROGRESS_CHARS`, `verify_release_digest = true` (under
`checksums`), and `verifying_keys = vec![]`.

`build()` (`common.rs:142-190`) validates and resolves into `CommonConfig`
(`common.rs:194-216`):

- First calls `self.request.check()` (`common.rs:150`), surfacing any deferred
  `request_header` conversion failure as `Error::InvalidHeader { source }`.
- Required (each missing field yields `Error::MissingField { field }` naming the
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

`RequestConfig` carries `timeout`, `headers`, `retries`, the retry-backoff
delays, `client` / `async_client` (injected transports), root certificates
(`add_root_certificate`), the auth fields (`auth_scheme`, `auth_token`,
`auth_base_host`, the `allow_auth_host` allowlist, the non-https-forwarding
flag), and `header_error`. `insert_header`
stays infallible, recording the first bad name/value in `header_error`;
`check` replays it as `Error::InvalidHeader { source }` and surfaces a
root-certificate/client-build failure as `Error::InvalidCertificate { source }`.

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
- `progress_style(ProgressStyle)` (`macros.rs:342`) - sets template and chars via
  the typed `ProgressStyle { template, chars }` newtype (`ProgressStyle::new(template, chars)`).
- `show_output(bool)` (`macros.rs:358`).
- `no_confirm(bool)` (`macros.rs:370`).
- `show_release_notes(bool)` - show the release notes URL (or the body when no URL is available)
  in the confirmation prompt; default off.
- `update_strategy(UpdateStrategy)` - `Compatible` (default, prefer the newest semver-compatible
  release, else newest overall) or `Latest` (always newest, across a major bump).
- `unattended()` (`macros.rs:378`) - one-call CI/daemon configuration: sets
  `no_confirm(true)` + `show_output(false)`. Without it the default
  (`no_confirm == false`) blocks on stdin waiting for confirmation.

`tag_prefix(impl Into<String>)` is NOT a shared setter: it is defined per-backend on the
github/gitlab/gitea `UpdateBuilder`s only (the tag-to-version derivation is forge-specific; s3
parses versions from object keys and the custom backend supplies its own `Release`s). It writes
`self.common.tag_prefix`, read by each forge's tag parser via `backends::common::strip_tag_prefix`.
- `request_config_setters!(common.request)` - splices in
  `timeout`, `request_header`, `retries`, `retry_backoff(base, max)`,
  `http_client(Arc<dyn HttpClient>)` (and `http_client_async` under `async`),
  the thin wrappers `reqwest_client`, `reqwest_async_client`, `ureq_agent`
  (each feature-gated, delegating to `http_client` / `http_client_async`),
  `add_root_certificate(Certificate)` (trust a private/internal CA; a malformed
  cert surfaces as `Error::InvalidCertificate` from `build()`),
  `allow_auth_host(host)` (authorize an extra host, e.g. an asset CDN, to receive
  the auth token), and `dangerously_allow_non_https_auth_forwarding()` (allow the
  token over http to a host-matched request) (`macros.rs:14-186`).
- `progress_callback(impl Fn(u64, Option<u64>) ...)` (`macros.rs:391`).
- `asset_matcher(impl Fn(&[ReleaseAsset]) -> Option<ReleaseAsset> ...)`
  (`macros.rs:405`).
- `verify_binary(impl Fn(&Path) -> Result<()> ...)` (`macros.rs:589`) - the post-update
  hook on the extracted binary; its doc records the full verification order
  (`verify_checksum` -> release digest -> signature/`verifying_keys` -> extract ->
  `verify_binary` -> replace), so it runs last. `Err(..) => bail` with
  `Error::VerificationRejected { reason }`.
- `verify_checksum(Checksum)` (under `checksums`).
- `verify_release_digest(bool)` (under `checksums`, default on) - toggles verifying the
  download against the selected asset's backend-published digest.
- `verifying_keys(impl Into<Vec<VerifyingKey>>)` (`macros.rs:617`, under
  `signatures`; renamed from `verify_keys`) - **replaces** the key set on each call
  (last call wins, unlike
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
`&Path` for `bin_install_path`; plain `bool`/`Copy` returns
for the toggles. The crate-private accessors (`macros.rs:226-263`) live on the
`pub(crate) trait UpdateInternals` (not the public `UpdateConfig`):
`request_timeout`, `request_headers`, `request_config`, `request_client`,
`request_async_client` (`async`), `progress_callback`,
`verify_callback`, `asset_matcher`, `verify_checksum` and `verify_release_digest`
(`checksums`), and `verify_keys` (`signatures`, reading the `verifying_keys` field). See
`update-config-internal-accessors.md`.

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
- `get_newer_releases_async()` - releases strictly newer than the current version (renamed from
  `get_latest_releases_async`).
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
  `bin_path_in_archive` with `Error::MissingField { field }` naming the setter to
  call, and replays any deferred `request_header` error as `Error::InvalidHeader { source }`
  before resolving defaults.
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

## Auth scheme, retry backoff, and progress style

- **Auth scheme.** `RequestConfig` carries `auth_scheme: AuthScheme`
  (`Token` for github/gitea, `Bearer` for gitlab) and `auth_token: Option<String>`,
  resolved from `CommonBuilderConfig` (and the git `ReleaseList` builders) at build
  time. A single derivation, `RequestConfig::apply_auth`, renders
  `"<scheme> <token>"` into the `Authorization` header on **both** the listing path
  (`send` / `send_async`) and the download path (`build_download`), and is skipped
  when the user supplied their own `Authorization` via `request_header` (the override
  wins on both paths). The token is host-gated: it is only attached to requests whose
  host matches the backend's configured API host (`auth_base_host`) or an
  `allow_auth_host` entry, over https. A server-supplied asset `download_url` or
  pagination `Link` pointing at a different host does not receive the token;
  `dangerously_allow_non_https_auth_forwarding()` lifts only the https requirement
  for a host-matched request. The per-backend `api_headers` overrides now only set the
  User-Agent; the `UpdateConfig::api_headers` trait default is a no-op.
- **Retry backoff.** `RequestConfig::{retry_base_delay, retry_max_delay}`
  (defaults 100ms / 3200ms) drive `retry_backoff_ms(attempt, base, max)`; set via the
  `retry_backoff(base, max)` builder setter.
- **ProgressStyle.** The two transposable `impl Into<String>` args of
  `progress_style` were replaced by a typed `ProgressStyle { template, chars }`
  newtype (`ProgressStyle::new(template, chars)`), threaded through the
  `progress_template` / `progress_chars` config fields. Behind the `progress-bar`
  feature.
- **UpdateInternals.** The crate-private-typed accessors moved off the public
  sealed `UpdateConfig` onto a `pub(crate) trait UpdateInternals`; see
  `update-config-internal-accessors.md`.

## Related

- `1.0-api-surface.md`
- `ref-release-model.md`
- `async-api.md`
- `update-config-internal-accessors.md`

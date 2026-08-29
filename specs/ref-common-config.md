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

`CommonBuilderConfig` (`src/backends/common.rs:CommonBuilderConfig`) is the pre-validation state held while a
backend's `UpdateBuilder` is configured. Each backend's builder embeds it as a
`common: CommonBuilderConfig` field (e.g. `src/backends/github.rs:UpdateBuilder`). Its fields
(`src/backends/common.rs:CommonBuilderConfig`): `request: RequestConfig`, `target`, `asset_identifier`,
`bin_name`, `bin_install_path`, `bin_path_in_archive`, `bin_path_in_archive_auto`
(`src/backends/common.rs:bin_path_in_archive_auto`, internal `bool` tracking whether `bin_path_in_archive` was
auto-derived from `bin_name`), `show_download_progress`, `show_output`,
`no_confirm`, `current_version`, `release_tag`, `progress_template`,
`progress_chars`, `auth_token`, `progress_callback`, `verify`, `asset_matcher`,
`checksum` and `verify_release_digest` (under `checksums`), and `verifying_keys`
(under `signatures`). (The full struct also carries `check_install_path_writable`,
`bundle_path_in_archive`, `bundle_install_path`, `show_release_notes`, `update_strategy`,
`tag_prefix`, `auth_token_from_env`, and `auth_scheme`, added by later features; see
`src/backends/common.rs:CommonBuilderConfig` for the complete, current field list.)

`Default` (`src/backends/common.rs:CommonBuilderConfig::default`) sets the non-`None` defaults:
`bin_path_in_archive_auto = false`, `show_download_progress = false`,
`show_output = true`, `no_confirm = false`,
`progress_template = DEFAULT_PROGRESS_TEMPLATE`,
`progress_chars = DEFAULT_PROGRESS_CHARS`, `verify_release_digest = true` (under
`checksums`), and `verifying_keys = vec![]`.

`build()` (`src/backends/common.rs:CommonBuilderConfig::build`) validates and resolves into `CommonConfig`
(`src/backends/common.rs:CommonConfig`):

- Bundle mode is resolved first (before any other field), by `resolve_bundle_mode`
  (`src/backends/common.rs:resolve_bundle_mode`), which returns the `(bundle_path_in_archive, bundle_install_path)` pair
  stored on `CommonConfig` (both `None` when `bundle_path_in_archive` is unset). With it set: an
  explicit `bin_install_path`, or a `bin_path_in_archive` whose `bin_path_in_archive_auto` is
  `false`, is `Error::ConflictingConfig { field, conflict }`; an unset `bundle_install_path`
  resolves through `update::default_bundle_install_path()` (macOS: the nearest `.app` ancestor of
  `current_exe()`, else `Error::NoAppBundle` / `Error::AppTranslocated`; other targets:
  `Error::MissingField { field: "bundle_install_path" }`).
- Then calls `self.request.check()` (`src/backends/common.rs:RequestConfig::check`), surfacing any deferred
  `request_header` conversion failure as `Error::InvalidHeader { source }`, and any
  root-certificate/client-build failure as `Error::InvalidCertificate { source }`.
- Required (each missing field yields `Error::MissingField { field }`, whose `Display` names the
  field generically as `` "`{field}` required" ``, per `ref-errors.md`): `current_version`
  (`src/backends/common.rs:current_version`), `bin_name` (`src/backends/common.rs:bin_name`), `bin_path_in_archive`
  (`src/backends/common.rs:bin_path_in_archive`). The last is normally set automatically by the `bin_name` setter, so
  callers only need set `bin_name`.
- Defaulted: `target` falls back to `get_target()` (`src/backends/common.rs:target`);
  `bin_install_path` falls back to `std::env::current_exe()` (`src/backends/common.rs:bin_install_path`),
  which can itself error and propagates via `?`.
- All other fields are cloned through unchanged. Note `target` and
  `current_version` become owned `String`, and `bin_install_path` an owned
  `PathBuf`, in `CommonConfig`.

`RequestConfig` carries `timeout`, `headers`, `retries`, the retry-backoff
delays, `client` / `async_client` (injected transports), root certificates
(`add_root_certificate`), the auth fields (`auth_scheme`, `auth_token`,
`auth_base_host`, the `allow_auth_host` allowlist, the non-https-forwarding
flag), and `header_error`. `insert_header` (`src/backends/common.rs:insert_header`)
stays infallible, recording the first bad name/value in `header_error`;
`check` replays it as `Error::InvalidHeader { source }` and surfaces a
root-certificate/client-build failure as `Error::InvalidCertificate { source }`.
`insert_header` also marks the value [`set_sensitive`](https://docs.rs/http/latest/http/header/struct.HeaderValue.html#method.set_sensitive)
when the header name is credential-bearing -- `authorization`, gitlab's `private-token`, `cookie`
(exact match), or any name ending in `-token` (`header_name_is_credential_bearing`,
`src/backends/common.rs:header_name_is_credential_bearing`) -- the same treatment `apply_auth` gives the derived `Authorization` header.
Without this, a user-supplied `request_header("Authorization", ..)` (which `apply_auth` gives
*precedence* over the backend's own token) would render verbatim in any `Debug` output, including
the one this struct and every backend builder inherit.

### Shared setter macro: impl_common_builder_setters!

`impl_common_builder_setters!` (`src/macros.rs:impl_common_builder_setters`) is invoked once inside each
backend's `impl UpdateBuilder` block (e.g. `src/backends/github.rs:UpdateBuilder`) and emits every shared
setter, each writing through `self.common.*` and returning `&mut Self`. Adding a
shared setter happens here once and reaches all backends.

Two invocation forms (a third, bare `()`, was removed as dead code once every backend settled on
one of these two -- see the Invariants note below): `(auth_env: ["VAR", ..], rationale: ..)`
emits the `@shared` set plus `auth_token` and an `auth_token_from_env()` setter for the named
environment variables (the form the github/gitlab/gitea/gitee `UpdateBuilder`s use); and
`(no_auth_token)` emits only `@shared`, for backends like s3 that authenticate differently.

`auth_token_from_env()` itself is emitted by the standalone `impl_auth_token_from_env!(token:
.., env_sourced: .., vars: [..], rationale: ..)` macro (`src/macros.rs:impl_auth_token_from_env`), which the
`ReleaseListBuilder`s (whose token is their own field, not `common.auth_token`) invoke directly.
It resolves the variables through `backends::common::token_from_env` -> `first_env_token` (first
present and non-empty after trimming wins) and writes the result with
`fill_env_token_if_unset_with(slot, resolve)` (`src/backends/common.rs:fill_env_token_if_unset_with`): a resolved token fills the
slot **only when it is currently blank** (unset, or holding only whitespace -- the blank-token
rule below), and `resolve` is not even called, let alone its diagnostics logged, when the slot is
already non-blank. `fill_env_token_if_unset(slot, resolved)` (`src/backends/common.rs:fill_env_token_if_unset`; renamed from
the earlier `apply_env_token`) is a thin wrapper over an already-resolved value, kept only so its
original unit tests stand -- the generated setter calls the `_with` form directly. Concretely: an
explicit `auth_token(..)` with a non-blank value always wins over `auth_token_from_env()`, in
either call order -- `auth_token(t).auth_token_from_env()` and `auth_token_from_env().auth_token(t)`
both end up with `t`, for non-blank `t` (a blank `t` is the documented exception to
order-independence; see the blank-token rule below). This supersedes an earlier "last-setter-wins
when the environment supplies a token" reading,
which let a later `auth_token_from_env()` call silently displace a token the application had
explicitly set; see `auth-token-from-env.md` AUTH-1-3 for the rationale. The explicit
`auth_token(..)` setter always overwrites the slot and clears an `auth_token_from_env` bookkeeping
flag, via the shared `set_explicit_auth_token(slot, env_sourced, value)` (`src/backends/common.rs:set_explicit_auth_token`,
called from `src/macros.rs:auth_token` and each hand-written `ReleaseListBuilder::auth_token`, e.g.
`src/backends/github.rs:ReleaseListBuilder::auth_token`), so a subsequent `auth_token_from_env()` call sees a filled slot and is a
no-op; the flag exists so `build()` can tell whether the *current* token came from the environment
(for the canonical-host decision below) even after an intervening call. The lookup happens in the
setter, not at request time. Variables per backend, in precedence order: github `GH_TOKEN`,
`GITHUB_TOKEN` (flipped from `GITHUB_TOKEN`-then-`GH_TOKEN` to match `gh help environment`'s
documented precedence); gitlab `GITLAB_TOKEN` only (`CI_JOB_TOKEN` was removed -- it is
project-scoped and not compatible with the `Authorization: Bearer` scheme this crate sends, so
keeping it meant `auth_token_from_env()` was not the advertised no-op inside GitLab CI); gitea
`GITEA_TOKEN`; gitee `GITEE_TOKEN`. Each list is also exposed as a crate-internal
`AUTH_TOKEN_ENV_VARS: &'static [&'static str]` constant (`src/macros.rs:AUTH_TOKEN_ENV_VARS`). Reading the environment
is opt-in: nothing reads it without this call. `has_auth_token() -> bool` (`src/macros.rs:has_auth_token`) is
emitted alongside it, reporting whether a non-blank token is set from either source without
exposing the value.

**Blank-token rule.** A blank token (empty, or all-whitespace) is treated as unset everywhere a
token is consulted, backed by one predicate, `is_blank_token(token: Option<&str>) -> bool`
(`src/backends/common.rs:is_blank_token`): it backs `fill_env_token_if_unset_with` above (so a blank explicit token
does not block the environment fallback), `RequestConfig::apply_auth` (`src/backends/common.rs:RequestConfig::apply_auth`, so a
blank token is never rendered into a literal `Authorization: token ` header), and
`has_auth_token()`. This is distinct from `first_env_token`'s trimming: an explicit
`auth_token(..)` value is never trimmed, so a token merely *surrounded* by whitespace still
surfaces as `Error::InvalidAuthToken` at request time.

A blank token is not order-independent, unlike a non-blank one: `set_explicit_auth_token`
overwrites the slot unconditionally, so `auth_token("").auth_token_from_env()` picks up the env
token (the blank slot lets the fallback through), while `auth_token_from_env().auth_token("")`
discards it (the explicit call clobbers whatever the fallback just filled). This is a deliberate,
kept tradeoff, not a bug: see `auth-token-from-env.md` AUTH-1-3 for the full asymmetry.

The `@shared` vocabulary (`src/macros.rs:impl_common_builder_setters`):

- `current_version(impl Into<String>)` (`src/macros.rs:current_version`) - required.
- `release_tag(impl Into<String>)` (`src/macros.rs:release_tag`) - used verbatim.
- `target(impl Into<String>)` (`src/macros.rs:target`).
- `asset_identifier(impl Into<String>)` (`src/macros.rs:asset_identifier`).
- `bin_name(impl Into<String>)` (`src/macros.rs:bin_name`) - required; appends `EXE_SUFFIX` if absent
  and (re-)derives `bin_path_in_archive` when that path is unset or was previously
  auto-derived, setting `bin_path_in_archive_auto = true`. Re-calling `bin_name` thus
  re-derives the archive path rather than leaving a stale one; an explicitly set
  `bin_path_in_archive` is sticky and is never overwritten.
- `bin_install_path<A: AsRef<Path>>(A)` (`src/macros.rs:bin_install_path`).
- `bin_path_in_archive(impl Into<String>)` (`src/macros.rs:bin_path_in_archive`) - supports `{{ bin }}`,
  `{{ target }}`, `{{ version }}` substitutions; sets `bin_path_in_archive_auto = false`
  so a later `bin_name` call will not overwrite it.
- `bundle_path_in_archive(impl Into<String>)` (`src/macros.rs:bundle_path_in_archive`) - names the bundle directory
  inside the archive and selects bundle mode; supports the same `{{ bin }}` / `{{ target }}` /
  `{{ version }}` substitutions as `bin_path_in_archive`.
- `bundle_install_path<A: AsRef<Path>>(A)` (`src/macros.rs:bundle_install_path`) - the installed bundle directory
  bundle mode replaces; optional on macOS (defaults to the nearest `.app` ancestor of the
  running exe), required elsewhere in bundle mode.
- `show_download_progress(bool)` (`src/macros.rs:show_download_progress`).
- `progress_style(ProgressStyle)` (`src/macros.rs:progress_style`) - sets template and chars via
  the typed `ProgressStyle { template, chars }` newtype (`ProgressStyle::new(template, chars)`).
- `show_output(bool)` (`src/macros.rs:show_output`).
- `no_confirm(bool)` (`src/macros.rs:no_confirm`).
- `update_strategy(UpdateStrategy)` (`src/macros.rs:update_strategy`) - `Compatible` (default, prefer the newest
  semver-compatible release, else newest overall) or `Latest` (always newest, across a major
  bump).
- `show_release_notes(bool)` (`src/macros.rs:show_release_notes`) - show the release notes URL (or the body when no
  URL is available) in the confirmation prompt; default off.
- `unattended()` (`src/macros.rs:unattended`) - one-call CI/daemon configuration: sets
  `no_confirm(true)` + `show_output(false)`. Without it the default
  (`no_confirm == false`) blocks on stdin waiting for confirmation.

`tag_prefix(impl Into<String>)` is NOT a shared setter: it is defined per-backend on the
github/gitlab/gitea `UpdateBuilder`s only (the tag-to-version derivation is forge-specific; s3
parses versions from object keys and the custom backend supplies its own `Release`s). It writes
`self.common.tag_prefix`, read by each forge's tag parser via `backends::common::strip_tag_prefix`.
- `request_config_setters!(common.request)` (invoked at `src/macros.rs:request_config_setters`) - splices in
  `timeout`, `request_header`, `retries`, `retry_backoff(base, max)`,
  `http_client(Arc<dyn HttpClient>)` (and `http_client_async` under `async`),
  the thin wrappers `reqwest_client`, `reqwest_async_client`, `ureq_agent`
  (each feature-gated, delegating to `http_client` / `http_client_async`),
  `add_root_certificate(Certificate)` (trust a private/internal CA; a malformed
  cert surfaces as `Error::InvalidCertificate` from `build()`),
  `allow_auth_host(host)` (authorize an extra host, e.g. an asset CDN, to receive
  the auth token), and `dangerously_allow_non_https_auth_forwarding()` (allow the
  token over http to a host-matched request); the macro itself is defined at `src/macros.rs:request_config_setters`.
- `progress_callback(impl Fn(u64, Option<u64>) ...)` (`src/macros.rs:progress_callback`).
- `asset_matcher(impl Fn(&[ReleaseAsset]) -> Option<ReleaseAsset> ...)`
  (`src/macros.rs:asset_matcher`).
- `verify_binary(impl Fn(&Path) -> Result<()> ...)` (`src/macros.rs:verify_binary`) - the post-update
  hook on the extracted binary; its doc records the full verification order
  (`verify_checksum` -> release digest -> signature/`verifying_keys` -> extract ->
  `verify_binary` -> replace), so it runs last. `Err(..) => bail` with
  `Error::VerificationRejected { reason }`.
- `verify_archive(impl Fn(&Path) -> Result<()> ...)` (`src/macros.rs:verify_archive`) - the
  pre-extraction hook on the downloaded archive, for an external attestation/signature check whose
  subject is the released file itself (`gh attestation verify`, `cosign verify-blob`). Runs after
  the built-in archive gates and before extraction (`verify_checksum` -> release digest ->
  signature/`verifying_keys` -> `verify_archive` -> extract -> `verify_binary` -> replace).
  `Err(..) => bail` with `Error::ArchiveVerificationRejected { reason }`, a distinct variant from
  the `verify_binary` hook's.
- `verify_checksum(Checksum)` (`src/macros.rs:verify_checksum`, under `checksums`).
- `checksum_from_asset(impl Into<String>)` (`src/macros.rs:checksum_from_asset`, under `checksums`) -
  name a sums asset of the same release (e.g. `SHA256SUMS`) to resolve the expected digest from,
  fetched over the same transport before the artifact download. Independent of the two gates above;
  a lookup that yields no digest is `Error::ChecksumSourceInvalid`.
- `verify_release_digest(bool)` (`src/macros.rs:verify_release_digest`, under `checksums`, default on) - toggles
  verifying the download against the selected asset's backend-published digest.
- `verifying_keys(impl Into<Vec<VerifyingKey>>)` (`src/macros.rs:verifying_keys`, under
  `signatures`; renamed from `verify_keys`) - **replaces** the key set on each call
  (last call wins, unlike
  `request_header` which appends); an empty set (or never calling it) leaves
  signature verification disabled, which is not an error.
- `auth_token(impl Into<String>)` (`src/macros.rs:auth_token`, the `auth_env:` form; a blank value is
  stored verbatim but treated as unset by `has_auth_token()`, `apply_auth`, and the env fallback).
- `auth_token_from_env()` (the `auth_env:` form only) - resolve the token from the backend's
  conventional environment variables; a no-op when none is set.

### Accessor macro: impl_update_config_accessors!

`impl_update_config_accessors!` (`src/macros.rs:impl_update_config_accessors`) emits a full
`impl crate::update::UpdateConfig for $t` block (e.g. `src/backends/github.rs:Update`) reading through
`self.common.*`. Bodies borrow, never own: `&str` for `current_version` (`src/macros.rs:current_version`),
`target` (`src/macros.rs:target`), `bin_name` (`src/macros.rs:bin_name`), `bin_path_in_archive` (`src/macros.rs:bin_path_in_archive`),
`progress_template` (`src/macros.rs:progress_template`), `progress_chars` (`src/macros.rs:progress_chars`, under `progress-bar`);
`Option<&str>` via `.as_deref()` for `release_tag` (`src/macros.rs:release_tag`), `asset_identifier`
(`src/macros.rs:asset_identifier`), `auth_token` (`src/macros.rs:auth_token`, reading `self.common.request.auth_token` -- the
*resolved* `RequestConfig`'s copy, the single source of truth `apply_auth` itself reads);
`&Path` for `bin_install_path` (`src/macros.rs:bin_install_path`); plain `bool`/`Copy` returns
for the toggles, including newer ones (`check_install_path_writable`, `bundle_path_in_archive`,
`bundle_install_path`, `update_strategy`, `show_release_notes`) added by later features, all in
the same `(@emit ...)` arm (`src/macros.rs:impl_update_config_accessors`). The crate-private accessors
(`(@internals ...)` arm, `src/macros.rs:impl_update_config_accessors`) live on the
`pub(crate) trait UpdateInternals` (not the public `UpdateConfig`):
`request_timeout`, `request_headers`, `request_config`, `request_client`,
`request_async_client` (`async`), `progress_callback`,
`verify_callback`, `verify_archive_callback`, `asset_matcher`, `verify_checksum`,
`checksum_from_asset` and `verify_release_digest`
(`checksums`), and `verifying_keys` (`src/macros.rs:verifying_keys`, `signatures`) -- the accessor and the
field it reads share the same name; there is no separate `verify_keys` accessor. See
`update-config-internal-accessors.md`.

Three invocation forms: bare `($t)` (`src/macros.rs:impl_update_config_accessors`) for the default
`api_headers`; `($t, { ... })` (`src/macros.rs:impl_update_config_accessors`) splices a custom `api_headers`
override into the same `impl` (github/gitlab/gitea); and `($t, where ( ... ))`
(`src/macros.rs:impl_update_config_accessors`) for the generic custom `AsyncUpdate<S>`.

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
largely `#[doc(hidden)]` plumbing. `has_auth_token()` (where emitted, i.e. every backend that has
`auth_token_from_env()`) is `pub`, reachable from user code; the `AUTH_TOKEN_ENV_VARS` list each
builder carries is `pub(crate)` -- not part of the public API, but the literal list the public
setter reads.

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
- `(auth_env: [..])` emits `auth_token` plus `auth_token_from_env` and `has_auth_token`;
  `(no_auth_token)` omits all three. A bare `()` form used to exist for a shared-setter caller
  with no env-var convention, but every backend now uses one of the two forms above, so it was
  removed as dead code.
- `auth_token_from_env()` never clears an existing token: with nothing set in the environment
  the builder's token is left exactly as it was.
- An explicit `auth_token(..)` with a non-blank value always wins over `auth_token_from_env()`,
  whatever the call order: the environment is a fallback that only fills a blank slot, never an
  override for a populated one. A blank explicit token is the documented exception: it does not
  block a *later* `auth_token_from_env()` call, but it does clobber a token `auth_token_from_env()`
  already filled if it comes *after* it.
- `CommonBuilderConfig` and `RequestConfig` each carry a hand-written `Debug` that redacts
  `auth_token` to `"<token>"`; neither derives `Debug` for that field.
- `token_from_env` reads via `std::env::var_os`, not `var`: a present-but-non-UTF-8 value is
  logged and treated as unset, not silently dropped like `var(..).ok()` would.
- `env_token_host_decision` acts only on an env-sourced token bound to an unacknowledged host (not
  the backend's canonical host, and not an `allow_auth_host` entry); an explicitly-set token is
  never flagged. github/gitlab/gitee (each has a canonical host) warn and still send it; gitea
  (no canonical host) withholds it instead, clearing the request's token so it goes out anonymous.
- A blank token (empty or all-whitespace) is treated as unset by `fill_env_token_if_unset_with`,
  `apply_auth`, and `has_auth_token()`, backed by one shared `is_blank_token` predicate.

## Tests

`src/backends/common.rs` unit tests (`mod tests`, starting `src/backends/common.rs:tests`):
`build_requires_current_version_bin_name_and_archive_path` (`src/backends/common.rs:build_requires_current_version_bin_name_and_archive_path`),
`build_resolves_target_and_install_path_defaults` (`src/backends/common.rs:build_resolves_target_and_install_path_defaults`),
`build_error_message_names_the_setter_for_current_version` (`src/backends/common.rs:build_error_message_names_the_setter_for_current_version`) and
`build_error_message_names_the_setter_for_bin_name` (`src/backends/common.rs:build_error_message_names_the_setter_for_bin_name`) asserting
the required-field errors name the setter to call, and the `insert_header` (`src/backends/common.rs:insert_header`) /
`check` (`src/backends/common.rs:check`) cases covering deferred invalid-name /
invalid-value errors, first-error-wins, and the ok path.

Env-token resolution (`common.rs` `mod tests`): `first_env_token_takes_the_first_present_value`,
`first_env_token_skips_empty_and_whitespace_values`, `first_env_token_trims_surrounding_whitespace`,
`first_env_token_returns_none_when_nothing_is_set` pin `first_env_token`'s precedence rules;
`fill_env_token_if_unset_keeps_an_explicit_token`, `fill_env_token_if_unset_fills_an_empty_slot`,
`fill_env_token_if_unset_keeps_an_existing_token_when_env_resolves_to_none`,
`fill_env_token_if_unset_leaves_an_empty_slot_empty` pin the "explicit token always wins, env only
fills an empty slot" rule (AUTH-1-3); `debug_redacts_the_auth_token_but_keeps_other_fields` pins the
`CommonBuilderConfig::fmt` redaction. Each backend module (`github.rs`, `gitlab.rs`, `gitea.rs`,
`gitee.rs`) separately asserts its own `AUTH_TOKEN_ENV_VARS` list and `has_auth_token()` behavior.

Blank-token rule: `fill_env_token_if_unset_fills_over_a_blank_explicit_token`,
`fill_env_token_if_unset_leaves_a_blank_token_blank_when_the_env_resolves_to_none`,
`apply_auth_treats_a_blank_token_as_unset` (`common.rs`). The lazy `fill_env_token_if_unset_with`
closure form: `fill_env_token_if_unset_with_does_not_call_the_resolver_when_the_slot_is_filled`,
`fill_env_token_if_unset_with_calls_the_resolver_when_the_slot_is_empty`,
`fill_env_token_if_unset_with_calls_the_resolver_when_the_slot_is_blank`. The canonical-host
decision (`env_token_host_decision`): `warns_and_sends_when_an_env_token_targets_an_unacknowledged_custom_host`,
`sends_silently_when_the_host_is_acknowledged_via_allow_auth_host`,
`no_action_for_an_explicitly_set_token_on_a_custom_host`,
`withholds_for_a_backend_without_a_canonical_host_and_no_acknowledgement`,
`sends_for_a_backend_without_a_canonical_host_once_the_host_is_acknowledged`.

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
- **Token resolution from the environment.** `token_from_env(names: &[&str]) -> Option<String>`
  (`src/backends/common.rs:token_from_env`) reads each candidate variable with `std::env::var_os` (not `var`), passing
  the raw `OsString` through `env_token_value` (`src/backends/common.rs:env_token_value`): a value that is present but
  not valid UTF-8 cannot become an HTTP header value anyway, so it is logged with `log::warn!` and
  treated as unset (rather than silently ignored the way `var(..).ok()` would, which would make a
  mangled variable indistinguishable from an absent one). The first present, non-empty,
  whitespace-trimmed value wins (`first_env_token`, `src/backends/common.rs:first_env_token`).
- **Blank-token rule.** `is_blank_token(token: Option<&str>) -> bool` (`src/backends/common.rs:is_blank_token`) treats
  `None` and a whitespace-only `Some` the same way: unset. It backs three call sites --
  `fill_env_token_if_unset_with` (so a blank explicit token does not block the env fallback),
  `apply_auth` (so a blank token never renders as a literal `Authorization: token ` header), and
  `has_auth_token()` -- so "is a token configured?" answers consistently everywhere.
- **Canonical-host decision.** `env_token_host_decision(env_sourced: bool, auth_base_host:
  Option<&str>, auth_hosts: &[String], canonical_host: Option<&str>) -> EnvTokenDecision`
  (`src/backends/common.rs:env_token_host_decision`; replaces the earlier `warn_if_env_token_off_canonical_host`, which had no
  `auth_hosts` parameter and returned a plain `bool`) is called from every forge backend's
  `build()` after `auth_base_host` is resolved. It first checks `host_is_acknowledged(host,
  auth_hosts, canonical_host)` (`src/backends/common.rs:host_is_acknowledged`): the host is the backend's canonical one, or
  one already passed to `allow_auth_host(..)` -- either way `EnvTokenDecision::Sent`, no warning.
  It also returns `Sent` when the token is not env-sourced at all. Otherwise it branches on
  whether the backend has a canonical host (`EnvTokenDecision`, `src/backends/common.rs:EnvTokenDecision`):
  github/gitlab/gitee (each a `CANONICAL_AUTH_HOST` const: `api.github.com` / `gitlab.com` /
  `gitee.com`) get `WarnedAndSent` -- a `log::warn!` naming both hosts, but the token is still
  attached, unchanged from before this round except that `allow_auth_host(..)` now silences the
  warning too. Gitea (always self-hosted, passes `None`) gets `Withheld` instead: a `log::warn!`
  naming the host and both remedies, and the caller clears `request.auth_token` so the request
  goes out anonymous while `build()` still returns `Ok`. See `ref-gitea-backend.md` and
  `auth-token-from-env.md` AUTH-1-8 for the full rationale. An explicitly-set token
  (`auth_token(..)`) is never flagged either way, since it is not env-sourced.
- **Redacting `Debug`.** Both config structs that can hold a raw token carry a hand-written `Debug`
  that redacts it to `"<token>"` (`None` renders `None`): `RequestConfig::fmt` (`src/backends/common.rs:RequestConfig::fmt`)
  and `CommonBuilderConfig::fmt` (`src/backends/common.rs:CommonBuilderConfig::fmt`). `CommonBuilderConfig` holds its own separate
  `auth_token: Option<String>` field (`src/backends/common.rs:auth_token`, distinct from `RequestConfig::auth_token`,
  which is only populated at `build()` time), so a derived `Debug` on it would print a live
  credential -- including one the application author never typed, since `auth_token_from_env()` can
  put an ambient CI credential there. `CommonBuilderConfig::fmt` also renders `auth_token_from_env`
  verbatim (it carries no secret), so a debug dump still answers "is a token set, and did it come
  from the environment?".
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

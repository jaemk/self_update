# Changelog

## [unreleased]
### Added
- A new `Releases` type, returned by the release-fetch methods, carrying the fetched releases plus
  the updater's current version. It has `all() -> &[Release]`, `latest() -> Option<&Release>`
  (newest first), `into_vec() -> Vec<Release>`, and `is_update_available() -> Result<bool>` (true
  when the latest release is strictly newer than the current version). The light pre-check is now
  `updater.get_latest_releases()?.is_update_available()` (sync) or
  `updater.get_latest_releases_async().await?.is_update_available()` (async), which fetches the
  release list once instead of fetching twice. `Releases` is re-exported at the crate root and is
  distinct from the per-backend `ReleaseList` builder.
- Async fetch parity (under the `async` feature) on the built updater:
  `get_latest_release_async()` and `get_latest_releases_async()` returning `Result<Releases>`, and
  `get_release_version_async()` returning `Result<Release>`.
- Documented paths for each backend's per-backend `ReleaseList` type
  (`self_update::backends::<github|gitlab|gitea|s3>::ReleaseList`); the four backends each have a
  distinct `ReleaseList` builder, so they are surfaced consistently rather than unified under one
  crate-root type.
- A new `examples/custom.rs` showing the custom backend: a minimal `ReleaseSource` impl driving a
  sync update, plus an async variant under the `async` feature.

### Changed (breaking)
- The custom-endpoint setter on the gitlab and gitea backends is now `url(...)` (was
  `instance_url(...)`), on both the `Update` and `ReleaseList` builders. github already used
  `url(...)`, so every git backend now names the setter the same.
- `Error::Zip`, `Error::Signature`, `Error::Json`, and `Error::SemVer` are now opaque: each wraps
  `Box<dyn std::error::Error + Send + Sync>` instead of the concrete `zip::result::ZipError` /
  `zipsign_api::ZipsignError` / `serde_json::Error` / `semver::Error`. Code that matched the inner
  dependency type must inspect it via `Error::source()` (or downcast the box) instead.
- `Error::NonUTF8` is renamed to `Error::SignatureNonUTF8` (`signatures` feature).
- `Status::uptodate()` and `UpdateStatus::uptodate()` are renamed to `is_up_to_date()`, and
  `Status::updated()` / `UpdateStatus::updated()` to `is_updated()` (matching predicate pair).
- The release-fetch surface on the sealed `ReleaseUpdate` trait now returns the new `Releases`
  type: `get_latest_release()` and `get_latest_releases()` return `Result<Releases>` (the latter no
  longer takes a `current_version` argument). The per-updater `is_update_available()` /
  `is_update_available_async()` checks are removed; call `is_update_available()` on the returned
  `Releases` instead (see Added).
- `Download`, `Extract`, `Move`, and `MoveAll` are now `#[non_exhaustive]`, as are each backend's
  concrete `Update` and `custom::AsyncUpdate` structs (the return types of `build_async()`).
- `Download::header()` now takes `TryInto<HeaderName>` / `TryInto<HeaderValue>` and returns a
  `Result`, so string literals work (`.header("Accept", "application/octet-stream")?`); an invalid
  header is reported instead of requiring a pre-parsed `HeaderName`/`HeaderValue`.
- The sealed `UpdateConfig` accessor `bin_install_path()` returns `&Path` (was an owned
  `PathBuf`). Only relevant if you named the return type.
- `backends::custom::Blocking`'s inner field is now private. Construct it with
  `Blocking::new(source)` and read the wrapped source via `into_inner()` / `as_inner()`.
- `DEFAULT_PROGRESS_TEMPLATE` and `DEFAULT_PROGRESS_CHARS` are no longer public (internal defaults
  only).
- The `AsyncReleaseSource` trait now enforces `Send` on its returned futures at the type level, so
  a non-`Send` impl fails to compile at the impl site rather than later at the spawn site. Existing
  `Send` impls are unaffected.

### Removed
- The per-updater `is_update_available()` / `is_update_available_async()` checks. They fetched the
  release list a second time; fetch once and call `is_update_available()` on the returned `Releases`
  instead (`updater.get_latest_releases()?.is_update_available()`).
- The s3 `auth_token` setter is removed. The S3 backend authenticates only by signing requests with
  `access_key` (AWS SigV4), so use `.access_key((id, secret))` (the `s3-auth` feature) for
  private-bucket access. `auth_token` remains on github/gitlab/gitea.

## [1.0.0]
First stable release. This version makes a number of breaking changes to clean up the
public API surface. Future `1.x` releases will remain backwards compatible.

> **Upgrading from 0.x?** See the [1.0 migration guide](docs/migrations/0.x-to-1.0-human.md)
> for a complete walkthrough of every breaking change (and an
> [agent-oriented version](docs/migrations/0.x-to-1.0.md) for automated tooling).

### Added
- S3 auth support (request signing for private buckets) behind the `s3-auth` feature
  ([#172](https://github.com/jaemk/self_update/pull/172)).
- Re-export the `http` crate as `self_update::http`, so consumers can name the header
  types accepted by `Download::header`/`replace_headers` (e.g.
  `self_update::http::header::ACCEPT`) without a separate `http` dependency.
- Re-export `ReleaseUpdate`, `UpdateStatus`, `Release`, and `ReleaseAsset` at the crate root
  (e.g. `self_update::ReleaseUpdate`) — the types returned by `update_extended()` / `fetch()`.
- Re-export `zipsign_api` and add a `self_update::VerifyingKey` type alias (under the
  `signatures` feature) so `verifying_keys(...)` callers need neither a direct
  `zipsign-api` dependency nor a hard-coded key length.
- `asset_identifier(...)` builder setter on the `gitlab` and `s3` `UpdateBuilder`s (it already
  existed on `github`/`gitea`), so every backend can disambiguate multiple matching assets.
- `compile_error!` guards that turn invalid feature combinations into a clear message:
  enabling both or neither of `reqwest`/`ureq`, or both `default-tls`/`rustls`.
- `#[must_use]` on every builder type.
- A no-op `s3` alias cargo feature, so `features = ["s3"]` resolves for symmetry with the other
  backends. The S3 backend itself needs no feature (it is always compiled); only private-bucket
  request signing needs a feature (`s3-auth`).
- Transport control on the `Update` and `ReleaseList` builders: `.timeout(Duration)` bounds
  every HTTP request the builder makes (release listing and, for `Update`, the download);
  `.request_header(name, value)` adds an extra header to every request (e.g. for a
  proxy/gateway); and `.retries(n)` retries a failed API request with exponential
  backoff. `Download::timeout(..)` provides a timeout for the standalone downloader (which
  already had `header`). Both the `reqwest` and `ureq` clients honor the
  `HTTP(S)_PROXY` / `NO_PROXY` environment variables.
- Download progress callback: `Download::progress_callback(|downloaded, total| ..)` and the
  same `.progress_callback(..)` on every `Update` builder, invoked as the download streams
  (`total` is `None` when the server sends no `Content-Length`). It is independent of the
  terminal progress bar, so GUI / headless / logging consumers can observe download progress.
- Checksum verification behind the `checksums` feature: `Update::configure()
  .checksum(Checksum::Sha256(hex))` (or `Checksum::Sha512(..)`) verifies the
  downloaded artifact against a known digest — e.g. one published in a `SHA256SUMS` file —
  before installing it. The hash algorithm is selected by the `Checksum` variant, which is
  `#[non_exhaustive]` so more algorithms can be added later.
- Post-update verification hook: `Update::configure().verify_with(|new_exe: &Path| -> bool ..)`
  runs on the freshly-extracted binary *before* it replaces the installed one; returning `false`
  aborts the update with nothing installed, so a broken release cannot replace a working binary.
  (Typical use: run `new_exe --version` and check the output.)
- Custom asset matching: `Update::configure().asset_matcher(|assets: &[ReleaseAsset]| ..)` overrides
  the built-in `target`/`identifier` substring selection with an arbitrary rule, for releases whose
  asset names the default heuristic can't express. Returning `None` fails the update with "no asset
  found".
- Transactional multi-file install: a new `MoveAll` primitive installs a set of `(source -> dest)`
  moves atomically — either all succeed, or on the first failure every applied move is rolled back,
  so a multi-file update (a binary plus sidecar libraries/resources) can't be left half-applied. A
  documented cookbook (`extract_into` the whole archive, then `MoveAll`) covers the multi-file /
  non-executable install case the single-binary `update()` flow doesn't.
- Custom HTTP client injection: `reqwest_client(reqwest::blocking::Client)`,
  `reqwest_async_client(reqwest::Client)` (under `async`), and `ureq_agent(ureq::Agent)` on the
  `Update`/`ReleaseList` builders (and `Download`) let you supply a pre-built client for full control
  over TLS/mTLS, connection pooling, redirects, and proxies, or to reuse an existing client. The
  injected client is used for both listing and download; `.request_header()`/`.retries()` still
  apply (and `.timeout()` for reqwest), while proxy-env and the TLS feature defer to your client.
  The selected client crate is re-exported (`self_update::reqwest` / `self_update::ureq`). This also
  reuses one client across paginated requests instead of rebuilding one per call.
- Async update API behind the `async` feature: every built-in backend's `Update` builder gains
  `build_async()` (returning a concrete `Update`) with async verbs `update_async()`,
  `update_extended_async()`, and `get_latest_release_async()`. The blocking API is unchanged and the
  async path reuses the same response parsers and the same extract/install tail (no logic fork);
  only the release listing and the download are async. It is tokio-only and reqwest-only (`async` is
  incompatible with `ureq`).
- Custom backends: a new public `ReleaseSource` trait (three fetch methods — `get_latest_release`,
  `get_latest_releases`, `get_release_version`) plus a `backends::custom::Update` builder let you
  update from a host the built-in backends don't cover (another forge, a private registry, a plain
  HTTP directory). You implement only *where releases come from*; the crate runs its usual
  compare → select-asset → download → verify → extract → install flow over your source. The
  `ReleaseUpdate` trait stays sealed. To support this, `ReleaseAsset::new` and a `Release::builder()`
  (`ReleaseBuilder`) make those `#[non_exhaustive]` types constructible by downstream code (also
  handy for building `Release` values in your own tests).
- Async custom backend (under the `async` feature): a public `AsyncReleaseSource` trait (the three
  fetches as `async fn`, mirroring `ReleaseSource`) and a generic `backends::custom::AsyncUpdate<S>`
  builder with `build_async()` / `update_async()` let you update from a natively-async source. A
  `backends::custom::Blocking` adapter wraps a `Clone` sync `ReleaseSource` so it can drive the async
  updater via `tokio::task::spawn_blocking`. No `async-trait` dependency: the updater is generic over
  the source, so the `async fn`s need no boxing.

### Changed
- **Builder vocabulary unified** (the `with_` prefix is dropped, and setter/accessor names line
  up):
  - the custom-endpoint setter is `url(...)` on every git backend: `github` (its API endpoint, was
    `with_url`), and `gitlab`/`gitea` (the instance base URL, was `with_host`);
  - the `ReleaseList` release filter is now `filter_target(...)` (was `with_target`/`target`),
    distinct from the build-target `target(...)` on the `Update` builder;
  - the s3 `UpdateBuilder::access_key_id` is now `access_key` (matching the
    `ReleaseListBuilder`; the setter takes the full `(id, secret)` pair);
  - the version-tag setter is `release_tag(...)` (was `target_version_tag`) and the
    asset-disambiguation setter is `asset_identifier(...)` (was `identifier`), on every `Update`
    builder — each now matching its `ReleaseUpdate` accessor of the same name;
  - the checksum setter is `checksum(...)` (was `verifying_checksum`), matching the `checksum()`
    accessor;
  - the `Update`/`Download` progress setters are `progress_callback(...)` and `progress_style(...)`
    (were `set_progress_callback`/`set_progress_style`).
- **`Download` setters renamed to match the `Update`/`ReleaseList` builders**: `set_timeout` →
  `timeout`, `set_header` → `header`, `set_headers` → `replace_headers` (it replaces the whole
  `HeaderMap`), `show_progress` → `show_download_progress`, `set_progress_callback` →
  `progress_callback`, `set_progress_style` → `progress_style`. The old names remain as
  `#[doc(alias)]`s.
- **`request_header(name, value)` now accepts `TryInto<HeaderName>`/`TryInto<HeaderValue>`** on the
  `Update` and `ReleaseList` builders, so `.request_header("X-Foo", "bar")` works (no
  `.parse().unwrap()`); an invalid header is surfaced as `Error::Config` from `build()` instead of
  panicking. Typed-argument call sites still compile.
- **`ReleaseUpdate` is now a sealed trait** — downstream code can call it (every backend's
  `build()` returns a `Box<dyn ReleaseUpdate>`) but can no longer implement it for foreign
  types.
- **`ReleaseUpdate` accessors return borrows**: `current_version`/`target`/`bin_name`/
  `bin_path_in_archive`/`progress_template`/`progress_chars` return `&str`, and
  `release_tag`/`asset_identifier`/`auth_token` return `Option<&str>` (were owned `String`/
  `Option<String>`); `api_headers` takes `Option<&str>`.
- **`ReleaseUpdate` accessors renamed to match their setters**: the `target_version` accessor is
  now `release_tag` and `identifier` is now `asset_identifier`.
- **`ReleaseSource` is sync-only; a separate `AsyncReleaseSource` trait drives the async custom
  updater.** `ReleaseSource` is the three sync fetch methods plus `Send + Sync` (no `Clone` bound).
  For a natively-async source, implement the new public `AsyncReleaseSource` trait (the same three
  fetches as `async fn`, clean names without an `_async` suffix) and drive it through
  `backends::custom::AsyncUpdate` + `build_async()`; to reuse a `Clone` sync `ReleaseSource` from the
  async API, wrap it in `backends::custom::Blocking` (which runs the sync fetches on
  `tokio::task::spawn_blocking`). `AsyncReleaseSource` is consumed through generics (`AsyncUpdate<S>`,
  never a `dyn` object), so its `async fn`s need no `async-trait`/boxing. See *Added* below.
- **`ReleaseUpdate` accessors moved to a sealed `UpdateConfig` supertrait** (`ReleaseUpdate:
  UpdateConfig`). All the getters (`current_version`, `target`, `bin_name`, `release_tag`,
  `asset_identifier`, `auth_token`, `api_headers`, the progress/transport/verify getters, …) now
  live on `self_update::UpdateConfig`; `ReleaseUpdate` keeps the three fetches plus
  `update`/`update_extended`. Calling an accessor on a `Box<dyn ReleaseUpdate>` is unchanged; a
  generic helper bounded `R: ReleaseUpdate` that calls an accessor needs `use self_update::UpdateConfig;`.
- **`#[derive(Clone)]` added to every `UpdateBuilder`** (github/gitlab/gitea/s3/custom), matching
  the already-`Clone` `ReleaseListBuilder`s, so a configured builder can be cloned before `build()`.
- **`ReleaseList::fetch` now takes `&self`** (was `self`) on the `github`/`gitlab`/`gitea`
  backends (the `s3` backend already borrowed).
- **`Error` is now `#[non_exhaustive]`**, and the feature-specific variants are collapsed:
  the `Reqwest`/`Ureq` variants are replaced by a single opaque `Http` variant, and the
  `StdTimeError`/`TimeError`/`Digest`/`UrlParse` (`s3-auth`) variants by a single opaque
  `S3Auth` variant. The underlying error is still reachable via `Error::source()`.
- `Status`, `ArchiveKind`, `Compression`, `UpdateStatus`, `Release`, and `ReleaseAsset` are
  now `#[non_exhaustive]`.
- `Download::progress_style` and each backend's `UpdateBuilder::progress_style` now
  accept `impl Into<String>`.
- The `Download::header` doc example now uses `self_update::http::header::ACCEPT`, so it
  is client-agnostic and self-contained.
- The `verifying_keys(...)` setter and accessor now use the `self_update::VerifyingKey` alias in
  their signatures (instead of the raw `[u8; zipsign_api::PUBLIC_KEY_LENGTH]` array).
- The s3 `AccessKey` credential type is now public and re-exported as
  `self_update::backends::s3::AccessKey` (under `s3-auth`), and is `#[non_exhaustive]` so a future
  credential field (e.g. an STS session token) can be added without a break. Build it via its
  `(id, secret)` `From` impls.
- Respect pagination URLs when fetching GitHub releases
  ([#179](https://github.com/jaemk/self_update/pull/179)).
- Print a short "up to date" message instead of nothing when no update is available
  ([#180](https://github.com/jaemk/self_update/pull/180)).

### Removed
- The s3 `UpdateBuilder::auth_token` setter. The S3 backend authenticates by signing requests with
  `access_key` (AWS SigV4), never a bearer token, so the setter never had any effect there. Use
  `.access_key((id, secret))` (the `s3-auth` feature) instead. `auth_token` remains on
  github/gitlab/gitea.
- The `Error::Reqwest`, `Error::Ureq`, `Error::StdTimeError`, `Error::TimeError`,
  `Error::Digest`, and `Error::UrlParse` variants (replaced by `Error::Http` /
  `Error::S3Auth`).
- The `self_replace` and `tempfile::TempDir` re-exports (`self_update::self_replace` /
  `self_update::TempDir`). They pinned consumers to the crate's exact dependency versions; depend
  on `self-replace` / `tempfile` directly instead. The `http`, `reqwest`/`ureq`, and `zipsign_api`
  re-exports are unchanged.
- `GetArchiveReaderResult` is no longer `pub` (it leaked `either::Either` /
  `flate2::read::GzDecoder` for a private function and had no consumer use).
- The deprecated `std::error::Error::description` implementation on `Error`.
- `should_update` (deprecated since 0.4.2) — use `version::bump_is_greater` or
  `version::bump_is_compatible` instead.
- The implicit setting of the `SSL_CERT_FILE` / `SSL_CERT_DIR` environment variables on Linux.
  The crate previously mutated these process-wide (to hardcoded Debian/Ubuntu paths) via
  `std::env::set_var`, which is unsound in a multi-threaded process and wrong on other distros. If
  the native-TLS (`default-tls`) backend can't find your CA bundle in a minimal environment, set
  those variables yourself before running, or build with `rustls` (see the crate "Troubleshooting"
  docs).

### Fixed
- `api_headers` no longer panics on an auth token that is not a valid HTTP header value;
  it returns `Error::Config` instead.
- Extracting a zip no longer panics on a non-UTF-8 file path; it returns an error.
- `gitlab`/`gitea` `get_latest_release` returns a clear "no releases found" error for an
  empty release list instead of a misleading "missing `tag_name`" parse error.
- Fixed a malformed format placeholder in an s3 request-signing error message.
- `update()` now paginates the release listing when searching for a compatible version
  (github/gitlab/gitea). Previously only the first page was scanned, so a compatible release
  beyond page one could be missed; `ReleaseList::fetch` already paginated, and both now share
  one bounded `Link: rel="next"` walk.

### Migration guide
The full walkthrough is in [`docs/migrations/0.x-to-1.0-human.md`](docs/migrations/0.x-to-1.0-human.md)
(and an [agent-oriented version](docs/migrations/0.x-to-1.0.md) for automated tooling). Mechanical
find/replace for the common cases:

- Custom endpoint setter (now `url(...)` on every git backend):
  - `.with_url(` → `.url(`  (github API endpoint, kept)
  - `.with_host(` → `.url(` (gitlab, gitea)
- Release-list target filter: `.with_target(` → `.filter_target(`
- S3 credentials: `.access_key_id(` → `.access_key(`
- Version tag / asset id setters: `.target_version_tag(` → `.release_tag(`, `.identifier(` →
  `.asset_identifier(`; checksum setter `.verifying_checksum(` → `.checksum(`
- `Download`/`Update` progress + transport setters: `.set_progress_callback(` →
  `.progress_callback(`, `.set_progress_style(` → `.progress_style(`, and on `Download`
  `.set_timeout(` → `.timeout(`, `.set_header(` → `.header(`, `.set_headers(` →
  `.replace_headers(`, `.show_progress(` → `.show_download_progress(`
- Dropped re-exports: replace `self_update::self_replace::` / `self_update::TempDir` with a direct
  `self-replace` / `tempfile` dependency.
- Error matching (the enum is now `#[non_exhaustive]`, so add a `_ =>` arm):
  - `Error::Reqwest(e)` / `Error::Ureq(e)` → `Error::Http(e)`
  - `Error::StdTimeError(e)` / `Error::TimeError(e)` / `Error::Digest(e)` / `Error::UrlParse(e)` → `Error::S3Auth(e)`
- `ReleaseUpdate` accessors now return borrows: append `.to_string()` where you previously
  got an owned `String` (e.g. `updater.current_version().to_string()`).
- Custom `impl ReleaseUpdate for MyType` is no longer possible (the trait is sealed); open
  an issue if you need a custom backend.
- Verifying keys: `[u8; zipsign_api::PUBLIC_KEY_LENGTH]` may now be written
  `self_update::VerifyingKey`.
- Feature flags: enabling both `reqwest` and `ureq`, or both `default-tls` and `rustls`, is
  now a compile error. For `ureq` or `rustls`, set `default-features = false` and select one
  client + one TLS backend.

## [0.44.0]
### Added
- *(s3)* support generic S3 endpoints ([#171](https://github.com/jaemk/self_update/pull/171))
### Changed
- *(s3)* fix reverse release ordering ([#173](https://github.com/jaemk/self_update/pull/173))
- *(deps)* update reqwest to 0.13 ([#175](https://github.com/jaemk/self_update/pull/175))
### Removed

## [0.43.1]
### Added
### Changed
- Improve `assert_for` logic to fallback to identifier-only search if
  target/os-arch search fails
- Fix update logic to respect `bin_install_path` when not equal to the
  current exe. Logic was previously modified to use the `self_replace`
  crate, but that change assumed the installation was always replacing
  the current exe.
### Removed

## [0.43.0]
### Added
- Docs: add documentation for [`self_update::errors::Error`]
### Changed
- Improve `assert_for` logic to prioritize searching by asset name and identifier
  before looking for assets by OS/arch
### Removed

## [0.42.0]
### Added
- Improved release search/lookup capability to support filtering assets by identifier
- Improved version specifications to support prerelease tags and parallel supported versions
### Changed
- Update reqwest features to allow http2 negotiation
- Update quick-xml (0.37) and zipsign (0.1)
- Specify per_page=100 when fetching github releases
### Removed

## [0.41.0]
### Added
### Changed
- Update to zip 2.x
### Removed

## [0.40.0]
### Added
### Changed
- `Release::asset_for` now searches for current `OS` and `ARCH` inside `asset.name` if `target` failed to match
- Update `reqwest` to `0.12.0`
- Update `hyper` to `1.2.0`
- Support variable substitutions in `bin_path_in_archive` at runtime
### Removed

## [0.39.0]
### Added
- Add `signatures` feature to support verifying zip/tar.gz artifacts using [zipsign](https://github.com/Kijewski/zipsign)
### Changed
- MSRV = 1.64
### Removed

## [0.38.0]
### Added
### Changed
- Use `self-replace` to replace the current executable
### Removed

## [0.37.0]
### Added
### Changed
- Bugfix: use appropriate auth headers for each backend (fix gitlab private repo updates)
### Removed

## [0.36.0]
### Added
### Changed
- For the gitlab backend, urlencode the repo owner in API calls to handle cases where the repo is owned by a subgroup
### Removed

## [0.35.0]
### Added
### Changed
- Support selecting from multiple release artifacts by specifying an `identifier`
- Update `quick-xml` to `0.23.0`
### Removed

## [0.34.0]
### Added
- Add `with_url` method to `UpdateBuilder`
### Changed
### Removed

## [0.33.0]
### Added
- Support for Gitea / Forgejo
### Changed
### Removed

## [0.32.0]
### Added
- Support for self hosted gitlab servers
### Changed
### Removed

## [0.31.0]
### Added
- Support S3 dualstack endpoints
### Changed
- Update `indicatif` 0.16.0 -> 0.17.0
### Removed

## [0.30.0]
### Added
### Changed
- Bump `semver` 0.11 -> 1.0
### Removed

## [0.29.0]
### Added
### Changed
- Bump `zip` 0.5 -> 0.6
- Bump `quick-xml` 0.20 -> 0.22
### Removed

## [0.28.0]
### Added
### Changed
- Bump indicatif 0.15 -> 0.16
### Removed

## [0.27.0]
### Added
### Changed
- Switch gitlab authorization header prefix from `token` to `Bearer`
### Removed

## [0.26.0]
### Added
### Changed
- Clean up dangling temporary directories on Windows.
### Removed

## [0.25.0]
### Added
### Changed
- Fix io error triggered when updating binary contained in a zipped folder.
- Fix issues updating Windows binaries on non-`C:` drives.
### Removed

## [0.24.0]
### Added
### Changed
- `UpdateBuilder.bin_name` will add the platform-specific exe suffix on the S3 backend.
### Removed

## [0.23.0]
### Added
### Changed
- update `reqwest` to `0.11`
- remove `hyper-old-types` dependency, replace the rel-link-header parsing
  with a manual parsing function: `find_rel_next_link`
### Removed

## [0.22.0]
### Added
### Changed
- bump dependencies
- print out tooling versions in CI
### Removed

## [0.21.0]
### Added
- Add GCS support to S3 backend
### Changed
- Fixed docs referring to github in s3 backend
### Removed

## [0.20.0]
### Added
- Add DigitalOcean Spaces support to S3 backend
### Changed
### Removed

## [0.19.0]
### Added
- Add `Download::set_header` for inserting into the download request's headers.
### Changed
- Update readme example to add `Accept: application/octet-stream` header. Release parsing
  was updated in 0.7.0 to use the github-api download url instead of the browser
  url so auth headers can be passed. When using the github-api download url, you
  need to pass `Accept: application/octet-stream` in order to get back a 302
  redirecting you to the "raw" download url. This was already being handled in
  `ReleaseUpdate::update_extended`, but wasn't added to the readme example.
### Removed

## [0.18.0]
### Added
- Allow specifying a custom github api url
### Changed
### Removed

## [0.17.0]
### Added
- Support for Gitlab
- Gitlab example
### Changed
- `UpdateBuilder.bin_name` will add the platform-specific exe suffix (defined
  by `std::env::consts::EXE_SUFFIX`) to the end of binary names if it's missing.
  This was a fix for windows.
### Removed

## [0.16.0]
### Added
### Changed
- switch from `tempdir` to `tempfile`
### Removed

## [0.15.0]
### Added
- Handling for `.tgz` files
### Changed
- Support version tags with or without leading `v`
- S3, support path prefixes that contain directories
### Removed

## [0.14.0]
### Added
- Expose `body` string in `Release` data
### Changed
### Removed

## [0.13.0]
### Added
- Feature flag `rustls` to enable using [rustls](https://github.com/ctz/rustls) instead of native openssl implementations.
### Changed
### Removed

## [0.12.0]
### Added
### Changed
- Make all archive and compression dependencies optional, available behind
  feature flags, and off by default. The feature flags are listed in the
  README. The common github-release use-case (tar.gz) requires the features
  `archive-tar compression-flate2`
- Make the `update` module public
### Removed

## [0.11.1]
### Added
### Changed
- add rust highlighting tag to doc example
### Removed

## [0.11.0]
### Added
### Changed
- set executable bits on non-windows
### Removed

## [0.10.0]
### Added
### Changed
- update reqwest to 0.10, add default user-agent to requests
- update indicatif to 0.13
### Removed

## [0.9.0]
### Added
- support for Amazon S3 as releases backend server
### Changed
- use `Update` trait in GitHub backend implementation for code re-usability
### Removed

## [0.8.0]
### Added

### Changed
- use the system temp directory on windows

### Removed

## [0.7.0]
### Added
### Changed
- accept `auth_token` in `Update` to allow obtaining releases from private GitHub repos
- use GitHub api url instead of browser url to download assets so that auth can be used for private repos
- accept headers in `Download` that can be used in GET request to download url (required for passing in auth token for private GitHub repos)
### Removed

## [0.6.0]
### Added
### Changed
- use indicatif instead of pbr
- update to rust 2018
- determine target arch at build time
### Removed


## [0.5.1]
### Added
- expose a more detailed `GitHubUpdateStatus`

### Changed
### Removed


## [0.5.0]
### Added
- zip archive support
- option to extract a single file

### Changed
- renamed github-updater `bin_path_in_tarball` to `bin_path_in_archive`

### Removed


## [0.4.5]
### Added
- freebsd support

### Changed

### Removed


## [0.4.4]
### Added

### Changed
- bump reqwest

### Removed


## [0.4.3]
### Added

### Changed
- Update readme - mention `trust` for producing releases
- Update `version` module docs

### Removed
- `macro` module is no longer public
    - `cargo_crate_version!` is still exported


## [0.4.2]
### Added
- `version` module for comparing semver tags more explicitly

### Changed
- Add deprecation warning for replacing `should_update` with `version::bump_is_compatible`
- Update the github `update` method to display the compatibility of new release versions.

### Removed

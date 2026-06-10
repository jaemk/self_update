# Changelog

## [unreleased]
### Added
### Changed
### Removed

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
  types accepted by `Download::set_header`/`replace_headers` (e.g.
  `self_update::http::header::ACCEPT`) without a separate `http` dependency.
- Re-export `ReleaseUpdate`, `UpdateStatus`, `Release`, and `ReleaseAsset` at the crate root
  (e.g. `self_update::ReleaseUpdate`) — the types returned by `update_extended()` / `fetch()`.
- Re-export `zipsign_api` and add a `self_update::VerifyingKey` type alias (under the
  `signatures` feature) so `verifying_keys(...)` callers need neither a direct
  `zipsign-api` dependency nor a hard-coded key length.
- `identifier(...)` builder setter on the `gitlab` and `s3` `UpdateBuilder`s (it already
  existed on `github`/`gitea`), so every backend can disambiguate multiple matching assets.
- `compile_error!` guards that turn invalid feature combinations into a clear message:
  enabling both or neither of `reqwest`/`ureq`, or both `default-tls`/`rustls`.
- `#[must_use]` on every builder type.
- Transport control on the `Update` and `ReleaseList` builders: `.timeout(Duration)` bounds
  every HTTP request the builder makes (release listing and, for `Update`, the download);
  `.request_header(name, value)` adds an extra header to every request (e.g. for a
  proxy/gateway); and `.retries(n)` retries a failed release-listing request with exponential
  backoff. `Download::set_timeout(..)` provides a timeout for the standalone downloader (which
  already had `set_header`). Both the `reqwest` and `ureq` clients honor the
  `HTTP(S)_PROXY` / `NO_PROXY` environment variables.
- Download progress callback: `Download::set_progress_callback(|downloaded, total| ..)` and the
  same `.set_progress_callback(..)` on every `Update` builder, invoked as the download streams
  (`total` is `None` when the server sends no `Content-Length`). It is independent of the
  terminal progress bar, so GUI / headless / logging consumers can observe download progress.
- Checksum verification behind the `checksums` feature: `Update::configure()
  .verifying_checksum(Checksum::Sha256(hex))` (or `Checksum::Sha512(..)`) verifies the
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

### Changed
- **Builder vocabulary unified** (the `with_` prefix is dropped):
  - the custom-endpoint setter is now `url(...)` on every git backend (was `with_url` on
    `github` and `with_host` on `gitlab`/`gitea`);
  - `ReleaseListBuilder::with_target` is now `target`;
  - the s3 `UpdateBuilder::access_key_id` is now `access_key` (matching the
    `ReleaseListBuilder`; the setter takes the full `(id, secret)` pair).
- **`ReleaseUpdate` is now a sealed trait** — downstream code can call it (every backend's
  `build()` returns a `Box<dyn ReleaseUpdate>`) but can no longer implement it for foreign
  types.
- **`ReleaseUpdate` accessors return borrows**: `current_version`/`target`/`bin_name`/
  `bin_path_in_archive`/`progress_template`/`progress_chars` return `&str`, and
  `target_version_tag`/`identifier`/`auth_token` return `Option<&str>` (were owned `String`/
  `Option<String>`); `api_headers` takes `Option<&str>`.
- **`ReleaseUpdate::target_version` accessor renamed to `target_version_tag`** so it matches the
  `target_version_tag` builder setter (the one remaining setter/accessor name mismatch).
- **`Download` header/progress setters renamed for clarity and cross-type consistency**:
  `set_headers` is now `replace_headers` (it discards and replaces the whole `HeaderMap`, unlike
  the additive `set_header`), and `show_progress` is now `show_download_progress` (matching the
  `Update` builder setter of the same name).
- **`ReleaseList::fetch` now takes `&self`** (was `self`) on the `github`/`gitlab`/`gitea`
  backends (the `s3` backend already borrowed).
- **`Error` is now `#[non_exhaustive]`**, and the feature-specific variants are collapsed:
  the `Reqwest`/`Ureq` variants are replaced by a single opaque `Http` variant, and the
  `StdTimeError`/`TimeError`/`Digest`/`UrlParse` (`s3-auth`) variants by a single opaque
  `S3Auth` variant. The underlying error is still reachable via `Error::source()`.
- `Status`, `ArchiveKind`, `Compression`, `UpdateStatus`, `Release`, and `ReleaseAsset` are
  now `#[non_exhaustive]`.
- `Download::set_progress_style` and each backend's `UpdateBuilder::set_progress_style` now
  accept `impl Into<String>`.
- The `Download::set_header` doc example now uses `self_update::http::header::ACCEPT`, so it
  is client-agnostic and self-contained.
- Respect pagination URLs when fetching GitHub releases
  ([#179](https://github.com/jaemk/self_update/pull/179)).
- Print a short "up to date" message instead of nothing when no update is available
  ([#180](https://github.com/jaemk/self_update/pull/180)).
- The s3 `UpdateBuilder::auth_token` setter is now `#[deprecated]` and a no-op: the S3 backend
  authenticates by signing requests with `access_key` (AWS SigV4), never a bearer token, so the
  setter has never had any effect there. Use `.access_key((id, secret))` instead. (It still
  exists for one release to avoid a hard break and will be removed in the next major version.)

### Removed
- The `Error::Reqwest`, `Error::Ureq`, `Error::StdTimeError`, `Error::TimeError`,
  `Error::Digest`, and `Error::UrlParse` variants (replaced by `Error::Http` /
  `Error::S3Auth`).
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

- Custom endpoint setter:
  - `.with_url(` → `.url(`   (github)
  - `.with_host(` → `.url(`  (gitlab, gitea)
- Release-list target filter: `.with_target(` → `.target(`
- S3 credentials: `.access_key_id(` → `.access_key(`
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

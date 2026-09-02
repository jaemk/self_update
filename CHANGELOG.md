# Changelog

## [unreleased]

### Added
- `proxy(url)` on every backend's `Update` / `ReleaseList` builder and on `Download`: route every
  request (release listing and asset download alike) through an HTTP proxy, with credentials
  allowed in the URL (`http://user:pass@proxy.corp:8080`) and sent to the proxy as
  `Proxy-Authorization`. `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` already covered the
  unauthenticated case; a proxy demanding a password previously forced callers to add `reqwest` or
  `ureq` as a direct dependency purely to build a client with a proxy on it. Applied to the same
  crate-built client as `add_root_certificate`, so an intercepting proxy that also needs a private
  CA is one client. An injected client (`http_client` / `reqwest_client` / `ureq_agent`) owns its
  own proxy config and is unaffected. On reqwest the configured proxy is applied alongside the env
  vars (first match wins); on a ureq-only build it replaces the env-var proxy (single proxy slot).
  Only HTTP CONNECT proxies are supported.
- `Error::InvalidProxy { source }`: an unparseable proxy URL, surfaced from `build()` /
  `download_to` / `download_to_async`. The password embedded in a proxy URL is redacted from this
  error (including from the wrapped client error) and from every `Debug` rendering of the config,
  so it cannot leak into logs.

### Changed

### Fixed
- `Extract::extract_file` (and so `bin_path_in_archive` on the update path) now finds an entry
  stored with a leading `./`. `tar -czf app.tar.gz -C dir .` names every entry that way, and the
  lookup was an exact match, so such an archive failed with "Could not find the required path in
  the archive" even though the file was present. A `./`-prefixed request now also matches a
  plainly-named entry. Only a leading `./` is ignored; interior components are still compared
  exactly, so a same-named file in a subdirectory cannot be selected by accident. Reported in #27.

### Removed

## [1.2.0]
Additive over `1.1.0`: one opt-in feature for the ureq client's trust store. No API breaks, no
migration needed.

### Added
- `native-certs` feature: the crate-built ureq client verifies against the OS trust store
  (`RootCerts::PlatformVerifier`) instead of Mozilla's bundled roots (`RootCerts::WebPki`). Needed
  behind a TLS-intercepting corporate proxy, whose CA is installed on the machine and is absent from
  the bundled set, so every request otherwise fails to verify. Off by default, so the ureq client's
  trust store is unchanged unless asked for. No effect on reqwest (its rustls setup already uses
  `rustls-platform-verifier`) or on an injected `ureq::Agent`, which owns its own TLS config.

### Changed

### Removed

## [1.1.0]
Additive over `1.0.0`: two verification entry points for releases the built-in gates do not cover.
No API breaks, no migration needed.

### Added
- `verify_archive(|archive: &Path| -> Result<()>)` on every backend's `Update` builder: a
  pre-extraction hook over the downloaded archive, for verification whose subject is the released
  file itself (`gh attestation verify`, `cosign verify-blob`). It runs after the checksum,
  release-digest, and signature gates and before extraction, in bundle mode as well. `verify_binary`
  sees the extracted binary, which has a different digest than the artifact a forge attested, so it
  could not host such a check. A rejection is the new
  `Error::ArchiveVerificationRejected { reason }`, distinct from `verify_binary`'s
  `Error::VerificationRejected`, and built by `Error::archive_verification_rejected(..)`.

- `checksum_from_asset(name)` on every backend's `Update` builder, under the `checksums` feature:
  name a sums asset of the same release (e.g. `SHA256SUMS`) and the updater fetches it before the
  artifact download and verifies the artifact against the entry for its file name.
  `Checksum::from_sums_file(sums, file_name)` is the parser behind it, accepting the coreutils text
  and binary modes, leading path components, the BSD tag form, `#` comments, and a whole-file bare
  digest, with the algorithm taken from the digest's length. A lookup that yields no digest is the
  new `Error::ChecksumSourceInvalid { asset, reason }`, never a silently skipped check. This is the
  digest source for gitlab / gitea / s3, whose APIs publish no per-asset digest.

## [1.0.0]
The stable 1.0. `1.x` releases from here are backwards compatible.

Upgrading from 0.x: see the [1.0 migration guide](docs/migrations/0.x-to-1.0-human.md) (and its
[agent-oriented version](docs/migrations/0.x-to-1.0.md) for automated tooling), which covers every
break across the release-candidate series.

Upgrading from a release candidate: the API is unchanged since rc.6, but rate-limited responses
are classified differently and credential handling is stricter. A 429, or a 403 reporting a spent
quota or carrying a usable `Retry-After`, is now `Error::RateLimited` rather than
`Error::Unauthorized`, so a `match` arm that keyed on `Unauthorized` for those cases needs a
`RateLimited` arm (use `err.rate_limit_delay()` for the wait). `retry` / `retry_async` return such
a response immediately instead of spending the retry budget. On gitea, a token resolved by
`auth_token_from_env()` is withheld unless the configured host was acknowledged with
`allow_auth_host(..)` or set explicitly with `auth_token(..)`. A blank `auth_token("")` is treated
as unset. See the entries below for the full list.

### Added
- `auth_token_from_env()` on the github/gitlab/gitea/gitee `Update` and `ReleaseList` builders
  (eight builders in all): read the token from the backend's conventional environment variables
  instead of plumbing `std::env::var` through the application. Reads `GH_TOKEN` then `GITHUB_TOKEN`
  (the `gh` CLI's documented precedence), `GITLAB_TOKEN`, `GITEA_TOKEN`, `GITEE_TOKEN`, using the
  first that is set and non-empty after trimming. An explicit `auth_token(..)` always wins, in
  either call order: the environment is only a fallback that fills an unset token, so an ambient
  `*_TOKEN` can never displace the credential the application provisioned. Opt-in: the crate never
  reads the environment on its own, since the configured API base can be a self-hosted host. With
  nothing set the call is a no-op and the request goes out unauthenticated, and it never clears a
  token. `CI_JOB_TOKEN` is deliberately not read on gitlab, even though every GitLab CI job exports
  it: this backend sends `Authorization: Bearer`, which is not GitLab's job-token mechanism (the
  `JOB-TOKEN` header / `job_token` parameter), so reading it would turn a working anonymous fetch of
  a public project into a 401/403 inside CI; pass it explicitly with `auth_token(..)` if you want
  it. The variable set does not change with `api_base_url` / `host`; `gh` reads
  `GH_ENTERPRISE_TOKEN` / `GITHUB_ENTERPRISE_TOKEN` for a GitHub Enterprise host and this crate does
  not, so an enterprise base url still needs one of the variables above (or an explicit token).
  ([#78](https://github.com/jaemk/self_update/issues/78))
- `has_auth_token()` on the same eight builders: whether an authorization token is configured, from
  either `auth_token(..)` or `auth_token_from_env()`. Reports presence only -- never validity, and
  never the value -- so an application can answer "am I about to run authenticated?" without
  reimplementing the environment-variable list.
- `Error::RateLimited { status, url, reset_at, retry_after }`: a spent request quota is now
  distinguished from a credential failure. A 429 is always rate limiting (with or without quota
  headers); a 403 is rate limiting when it carries a zero remaining-quota header
  (`x-ratelimit-remaining` / gitlab's `RateLimit-Remaining`) or a usable `Retry-After` (GitHub's
  *secondary* rate limit, which answers 403 + `Retry-After` while the primary quota is still
  nonzero). A 403 with neither stays `Error::Unauthorized`. Both server-supplied wait values are
  clamped to a 24h ceiling and resolve to `None` beyond it. `http_status()` and `url()` cover the new
  variant, and `Error::http_status_error_with_headers(status, url, &HeaderMap)` gives a custom
  `HttpClient` the same classification (the header-blind `Error::http_status_error` cannot see the
  quota headers and so never produces `RateLimited`). All three built-in client lanes -- reqwest,
  ureq, and an injected `ureq::Agent` -- classify identically.
  ([#78](https://github.com/jaemk/self_update/issues/78))
- `Error::rate_limit_delay() -> Option<Duration>`: how long to wait before retrying a `RateLimited`
  request, measured from now. Prefers the server's `Retry-After` and otherwise derives the wait from
  `reset_at` minus the current time; `None` when the window has already elapsed or nothing is known.
  This is the accessor to back off with: on GitHub's *primary* rate limit only `x-ratelimit-reset` is
  sent, so reading the raw fields and calling `retry_after.unwrap_or_default()` sleeps zero and burns
  more quota.

- Directory-bundle installs (macOS `.app`): `bundle_path_in_archive(..)` names the bundle directory
  inside the release archive and selects bundle mode, where the whole tree replaces
  `bundle_install_path(..)` instead of one file replacing `bin_install_path`. The new bundle is
  staged in the destination's parent and swapped by rename with the displaced tree stashed, so a
  failure restores the original bundle; a running executable inside the bundle is renamed aside
  first, so its path holds the new executable afterwards and composes with `restart()`. On macOS
  `bundle_install_path` defaults to the nearest `.app` ancestor of the running executable. The
  `verify_binary` hook receives the staged bundle root, and the opt-in
  `check_install_path_writable` preflight probes the bundle's parent directory. Adds
  `Error::NoAppBundle` (no `.app` ancestor to derive the path from), `Error::ConflictingConfig`
  (bundle mode combined with an explicit `bin_install_path` / `bin_path_in_archive`), and
  `Error::AppTranslocated` (a quarantined app running from a read-only translocated mount). A
  symlinked `bundle_install_path` is resolved first, so the tree behind the link is replaced and the
  link survives; `bundle_install_path` without `bundle_path_in_archive` is a `MissingField` error
  rather than a silently discarded path.
  ([#145](https://github.com/jaemk/self_update/issues/145))
- `compression-tar-xz` feature: decode `.tar.xz` / `.txz` archives and plain `.xz` single-file
  assets (pure-Rust `lzma-rs`, no C `liblzma` dependency, so it cross-compiles like the rest of the
  default stack). Opt-in, mirroring `compression-tar-gz`. Adds `Compression::Xz`.
  ([#143](https://github.com/jaemk/self_update/issues/143))
- `self_update::verify_signature(archive_path, keys)`: run the same embedded-signature check
  `update()` performs, standalone, for a caller that stages a download itself (e.g. an installer
  fetching a companion binary before the update loop exists). Takes `impl AsRef<Path>` and a
  `&[VerifyingKey]` slice; any-of key semantics, `.tar.gz` / `.zip` only (`signatures` feature).
  ([#150](https://github.com/jaemk/self_update/issues/150))
- `native-tls-vendored` feature: build OpenSSL from source and link it statically, for targets
  where a usable system OpenSSL is awkward (musl, some cross-compiles). Implies `native-tls`;
  applies to the reqwest client. ([#108](https://github.com/jaemk/self_update/issues/108))
- `UpdateStrategy` and the `update_strategy(..)` builder setter: control which release the unpinned
  "latest" path installs when several are newer. `Compatible` (default) prefers the newest
  semver-compatible release, falling back to the newest overall; `Latest` always selects the newest
  release, even across a major bump. ([#152](https://github.com/jaemk/self_update/issues/152))
- `Release::release_notes_url()` and `ReleaseBuilder::release_notes_url(..)`: the release page URL,
  filled by the github/gitlab/gitea backends from the release's `html_url` (`_links.self` for
  gitlab); `None` for s3. The `show_release_notes(bool)` builder setter shows it (or the release
  body when no URL is available) in the confirmation prompt.
  ([#148](https://github.com/jaemk/self_update/issues/148))
- `tag_prefix(..)` on the github/gitlab/gitea `Update` builders: derive the version from a
  monorepo-style tag such as `myapp-1.2.3` (or `myapp-v1.2.3`). Defaults to unset, which trims a
  leading `v` as before; when set, tags without the prefix are skipped from the listing rather than
  mis-parsed. ([#76](https://github.com/jaemk/self_update/issues/76))
- `asset_key_pattern(..)` on the s3 `Update`/`ReleaseList` builders: a custom regex for deriving
  `(name, version)` from object keys, replacing the built-in matcher whose version group only
  captures a `major.minor.patch` triple. Lets a pre-release key such as
  `mybin-0.1.2-beta-x86_64-unknown-linux-gnu` parse as `0.1.2-beta` instead of `0.1.2`. The
  pattern must define `name` and `version` named capture groups and is validated at `build()`
  (`Error::InvalidAssetKeyPattern`); a captured version that does not parse as semver skips the
  key. Unset keeps the existing matcher unchanged.
  ([#61](https://github.com/jaemk/self_update/issues/61))
- `self_update::restart` module: `restart()` and `restart_with(args)` relaunch the (already
  replaced) executable after an update so a long-running process picks up the new binary
  immediately. `restart()` reuses the current arguments; `restart_with(args)` supplies a fresh
  argument list (e.g. to drop an `--upgrade` flag so the restarted process does not update again).
  On unix the process image is replaced with `exec`; on windows the new binary is spawned and the
  current process exits. No feature gate, no new dependencies.
  ([#62](https://github.com/jaemk/self_update/issues/62))
- `self_update::check_interval::UpdateCheckGuard`: a small stamp-file guard that throttles how often
  an application checks for updates. `should_check()` reports whether the configured interval has
  elapsed since the last recorded check (a missing, corrupt, or future-dated stamp counts as due);
  `record_check()` stamps the current time via a write-to-temp-then-rename so a concurrent reader
  never sees a partial stamp. The caller owns the stamp-file path; it is a guard, not a scheduler,
  and pulls in no time/date dependency. ([#79](https://github.com/jaemk/self_update/issues/79))
- Docs: an "Authentication" section in the crate docs covering both token setters, every backend's
  environment variables, the explicit-token precedence rule, `has_auth_token()`, and the fact that
  the variable set does not change with the configured host. Authentication is cross-backend, so it
  is no longer buried under the rate-limit heading where only a github reader would find it.
  ([#78](https://github.com/jaemk/self_update/issues/78))
- Docs: a "GitHub rate limits" section in the crate docs covering GitHub's 60/hour (unauthenticated)
  vs 5000/hour (authenticated) per-source-IP API limits, which responses classify as
  `Error::RateLimited`, backing off with `rate_limit_delay()` (with a worked example), that the retry
  loop short-circuits rather than spending the budget, and how to mitigate (a token, and checking
  less often via `UpdateCheckGuard`). The "Custom HTTP client" section points a custom transport at
  `Error::http_status_error_with_headers`, since the header-blind `Error::http_status_error` can
  never report a rate limit. ([#78](https://github.com/jaemk/self_update/issues/78))
- Docs: the Features section now names the exact `no HTTP client selected` compile error a
  client-less build (e.g. `default-features = false, features = ["rustls"]`) produces, and shows the
  fix (add a client, e.g. `features = ["ureq", "rustls", "github"]`).
  ([#168](https://github.com/jaemk/self_update/issues/168))
- `check_install_path_writable(bool)` builder setter (default `false`) and
  `Error::InstallPathNotWritable { path: PathBuf }`: opt-in preflight that probes `bin_install_path`
  writability before the download; only a definite `PermissionDenied` refusal errors, indeterminate
  results proceed. The install step also raises this error on a permission failure, always naming
  the path. ([#112](https://github.com/jaemk/self_update/issues/112))
- `backends::manifest` (`manifest` feature): fetch and install releases from a static
  `manifest.json` served by any HTTP endpoint, with no forge-specific API. `ManifestSource`
  implements `ReleaseSource` (and `AsyncReleaseSource` under `async`); the facade
  `Update::configure()` wraps it with the standard update pipeline. Release entries with a
  non-semver `version` are skipped with a debug log. Relative asset `url` values resolve against
  the manifest URL's directory (truncated at the last `/`). An asset `digest` field
  (`sha256:<hex>`) maps to `ReleaseAsset::digest()` and plugs into the existing release-digest
  verification path (`checksums` feature). No new dependencies.
  ([#74](https://github.com/jaemk/self_update/issues/74))
- `gitee` backend: `backends::gitee::ReleaseList`, `Update`, and `AsyncUpdate` for Gitee releases,
  mirroring the `gitea` backend. Default host `https://gitee.com` with an optional `.host()` setter
  for enterprise instances. Bearer-token auth via `auth_token`. Nameless source-archive assets are
  skipped with a debug log rather than erroring.
  ([#121](https://github.com/jaemk/self_update/issues/121))

### Changed
- **A rate-limited response no longer returns `Error::Unauthorized`.** A 429, a 403 whose headers
  report a spent quota (`x-ratelimit-remaining: 0` / gitlab's `RateLimit-Remaining: 0`), or a 403
  carrying a usable `Retry-After` now returns `Error::RateLimited` instead. `Error` is
  `#[non_exhaustive]`, so this does not break compilation: code that matched
  `Unauthorized { status: 403, .. }` to detect rate limiting (which the previous docs told users to
  write) keeps compiling and silently stops matching, falling through to the wildcard arm. Migration:
  match `Error::RateLimited { .. }` instead, and take the wait from `rate_limit_delay()` rather than
  the fields.

  ```rust
  // before
  Err(Error::Unauthorized { status: 403, .. }) => back_off(),
  // after (write the variant with a trailing `..`; both wait fields are `Option`s)
  Err(err @ Error::RateLimited { .. }) => match err.rate_limit_delay() {
      Some(wait) => std::thread::sleep(wait),
      None => reschedule(),
  },
  ```

  A bare 403 with no quota signal is still `Error::Unauthorized`, so a genuine credential failure is
  unchanged. ([#78](https://github.com/jaemk/self_update/issues/78))
- `retry` / `retry_async` no longer retry a rate-limited request. An `Error::RateLimited` returns
  immediately regardless of the configured `retries`, instead of spending the budget on a quota that
  is already at zero (and, on GitHub's unauthenticated per-IP budget, shared with everyone behind the
  same egress IP). Every other error still consumes the budget as before. The download path retries
  through the same loop, so it short-circuits too.
- Each backend `Update` / `ReleaseList` builder's `Debug` output redacts the authorization token,
  rendering it as `"<token>"` instead of the value, so logging a builder no longer prints an ambient
  CI credential. All other fields are still shown.
- A credential passed via `request_header("Authorization", ..)` (or `PRIVATE-TOKEN`, `Cookie`, or
  any header name ending in `-token`, case-insensitive) is now marked sensitive, so it is redacted
  in a builder's `Debug` output and kept out of the underlying HTTP client's own header logging, the
  same as a token set with `auth_token(..)`. Previously only the `auth_token` slot was redacted, so
  a credential passed as a header printed verbatim.
- `build()` logs a `log::warn!` when a token resolved from the environment would be sent to a host
  other than the backend's canonical one (`api.github.com`, `gitlab.com`, `gitee.com`). The
  environment variables are conventions of the backend's own service, so an application that exposes
  its update url as configuration would otherwise hand `GITHUB_TOKEN` to an arbitrary host with no
  signal. An explicitly-set token is the application's own decision and is never warned about. gitea
  has no canonical host, so its rule is stricter: an env-sourced token is withheld at `build()`
  rather than sent, and the request goes out anonymous, unless the configured host was acknowledged
  by passing it to `allow_auth_host(..)` or by setting the token explicitly with `auth_token(..)`;
  the warning still fires, naming the host and the remedy. github/gitlab/gitee are unchanged: an
  env-sourced token to a non-canonical host still warns and is still sent. On every backend, a host
  passed to `allow_auth_host` no longer produces that warning.
- A blank (empty or all-whitespace) `auth_token(..)` is now treated as unset: it no longer blocks the
  `auth_token_from_env()` fallback, no longer sends an empty `Authorization` header, and
  `has_auth_token()` reports `false` for it.
- `Error::http_status_error(429, url)` returns `Error::RateLimited` (with both wait fields `None`)
  instead of `Error::HttpStatus`, so a custom `HttpClient` that has no headers to hand over still
  reports a 429 as rate limiting. 429 does not need a header to mean "too many requests" (RFC 6585);
  401/403 are unchanged on that path, since only a header distinguishes a spent quota from a
  credential failure. Use `Error::http_status_error_with_headers` to get the full classification.
- A `Retry-After: 0` is no longer treated as a rate-limit signal. A bare 403 carrying a zero
  `Retry-After` and no quota header stays `Error::Unauthorized` instead of becoming a zero-wait
  `Error::RateLimited`.
- An injected `ureq::Agent` classifies a non-2xx response exactly like the crate-built agents. The
  agent keeps ureq's default `http_status_as_error(true)`, whose `StatusCode` error carries no
  headers, so that path could not see the quota headers; the client now applies a per-request
  `http_status_as_error(false)` override, and all three client lanes (reqwest, ureq, injected ureq)
  produce the same `NotFound` / `Unauthorized` / `RateLimited` / `HttpStatus` mapping. Nothing else
  about the injected agent's timeout / TLS / proxy configuration is touched.
- A recognized-but-unsupported compression extension now fails loudly instead of silently
  installing the still-compressed bytes as the binary: a `.tar.xz` / `.txz` / `.xz` asset without
  the `compression-tar-xz` feature returns `Error::CompressionNotEnabled("xz")` (matching the
  existing `.gz` handling), rather than writing the compressed archive to the install path.
  ([#143](https://github.com/jaemk/self_update/issues/143))
- Install-step IO failures now name the install path: `PermissionDenied` becomes
  `Error::InstallPathNotWritable { path }` and any other IO error becomes `Error::Io` with the
  install path embedded in the message (the `ErrorKind` is preserved).
  ([#112](https://github.com/jaemk/self_update/issues/112))

### Fixed
- Zip extraction (`Extract::extract_into`) now restores symlink entries as real symlinks on unix
  instead of writing the link's target path out as a regular file. Materializing the target string
  corrupted directory trees that rely on symlinks (for example a macOS `.app` bundle whose
  `Frameworks/*/Versions/Current` links are load-bearing for the code signature), so a signed app
  extracted from a zip failed to launch. Tar extraction already handled symlinks correctly. A
  symlink target that would escape the extraction root (an absolute target, or a relative one whose
  `..` components resolve above the destination) is rejected, matching the existing zip-slip defense
  on entry names. That per-entry check is lexical, so as a backstop every zip entry's physical
  parent is canonicalized after its directories are created and must equal the canonical extraction
  root joined with the entry's lexical parent; this rejects a symlinked-parent traversal (an entry
  `d/sl -> ..` followed by `d/sl/evil -> ../../x`, lexically in-bounds but physically above the
  root) that the lexical check alone cannot catch, while descent through real directories is
  unaffected. On windows, where creating symlinks needs elevated privileges, symlink entries
  keep the previous regular-file behavior. `Extract::extract_file` now errors on a symlink entry
  rather than writing its target string out as the requested file.
- The update confirmation block printed the install path with `{:?}`, and `Path`'s `Debug` impl
  quotes the path and escapes each separator, so on windows the "Current exe" line read
  `"C:\\Users\\me\\bin\\app.exe"`, doubled backslashes the user never typed. The `Current exe` and
  `Current bundle` lines now print through `Path::display()`, so the path appears exactly as the
  platform writes it. The `New exe release` / `New exe download url` lines are strings rather than
  paths and keep their existing quoted form.
  ([#201](https://github.com/jaemk/self_update/pull/201))
- A server-supplied asset name containing a control character is now rejected as
  `Error::InvalidAssetName` alongside the existing empty / `.` / `..` / separator / absolute-path
  cases. The name is remote-controlled and is echoed into the confirmation block, so a `\r` or an
  ESC sequence in it could repaint or hide the lines (including the download url) that the user
  reads before authorizing the replacement.
- In bundle mode, a symlinked `bundle_install_path` is resolved once, before the confirmation
  prompt, and that single resolved path is what the status block names, what the
  `check_install_path_writable` preflight probes, and what the swap replaces. It was resolved again
  after the prompt, so repointing the link in between would have redirected the replacement to a
  tree the user never approved.
- Rollback failures log paths through `Path::display()` rather than `{:?}`, so a windows path in
  those messages keeps single separators. Diagnostic `debug!` logs keep `{:?}`, which renders a
  non-UTF-8 path unambiguously.

## [1.0.0-rc.6]
Additive over rc.5: automatic verification against github's per-asset release digests, default
`ReleaseSource` trait methods, and assorted constructors; plus non-semver-tag skipping in the
forge listings and a unified User-Agent. No breaking changes vs rc.5, so no migration is needed.

### Added
- Release-published digest verification (`checksums` feature): github publishes a `sha256:<hex>`
  digest per release asset, and the updater now verifies the downloaded artifact against it
  before installing whenever the selected asset carries one. On by default with the `checksums`
  feature; opt out with `verify_release_digest(false)` on the builders. A digest that is present
  but malformed or uses an unsupported algorithm fails the update rather than silently skipping.
  Independent of `verify_checksum` (when both apply, both must pass), and an integrity check
  only -- the forge recomputes the digest when an asset is replaced, so it is not a substitute
  for the `signatures` feature. ([#159](https://github.com/jaemk/self_update/issues/159))
- `ReleaseAsset::digest()`: the asset's content digest in `algorithm:hex` form, when the backend
  publishes one (github fills it; gitlab/gitea/s3 have none). `ReleaseAsset::with_digest(..)`
  attaches one when building assets in a custom `ReleaseSource`.
- `Checksum::parse_digest("sha256:<hex>")`: parse an `algorithm:hex` digest string (the form
  forges publish) into a `Checksum`; `sha256` and `sha512` are supported.
- `ReleaseSource` / `AsyncReleaseSource`: `get_latest_release` and `get_release_version` have
  default implementations derived from `get_releases` (newest-by-semver pick and exact version
  match, both order-independent), so a custom source only has to implement `get_releases`.
  Existing implementations are unaffected; override the defaults when the host has cheaper
  dedicated endpoints. New trait methods will only be added with defaults, so implementations
  keep compiling across minor releases.
- `Releases::with_current_version(v)`: attach a current version to an already-fetched listing
  (e.g. from `ReleaseList::fetch`) so `is_update_available()` works, without rebuilding via
  `into_vec` / `from_releases`.
- `Error::transport(source)`: build the `Transport` variant from an error value or a message
  string, for custom `HttpClient` / `AsyncHttpClient` implementations.
- `ReleaseBuilder::new()`, equivalent to `Release::builder()`.

### Changed
- `ReleaseBuilder::build()` validates that the version parses as bare semver and errors with
  `Error::SemVer` otherwise (a `v` prefix or a non-semver tag previously built fine and was
  silently skipped, or errored opaquely, later in the update pipeline). The github/gitlab/gitea
  listings skip releases whose tag is not semver after trimming a leading lowercase `v` -- e.g.
  a rolling `nightly` or `latest` tag alongside normal releases -- matching the pre-1.0 behavior
  of ignoring them; each skip is logged at `log::debug!` (enable e.g. `env_logger` with
  `RUST_LOG=self_update=debug` to see which tags were dropped). Fetching such a tag directly
  (`release_tag`) errors with `Error::SemVer` naming the offending tag, with the original parse
  failure on the `source()` chain. github's `get_latest_release` uses the API's dedicated
  `/releases/latest` endpoint and also errors (naming the tag) if that designated release is not
  semver; gitlab/gitea derive "latest" from the listing, skipping unparseable tags.
- All requests send the same `self-update/<version>` User-Agent when the caller has not set one
  via `request_header`. Previously github sent `rust/self-update` and gitlab/gitea and the
  standalone `Download` sent `rust-reqwest/self-update` (wrong under the `ureq` client).
- `ProgressStyle` is `#[non_exhaustive]`: construct with `ProgressStyle::new(template, chars)`
  instead of a struct literal. Field reads are unaffected.
- The `ArchiveNotEnabled` Display message matches the other variants' style (lowercase after the
  prefix, no trailing punctuation).

### Fixed
- The `ReleaseSource` / `AsyncReleaseSource` docs told implementors to construct error variants
  with struct literals, which does not compile downstream (the variants are `#[non_exhaustive]`);
  they now reference the public constructors (`Error::http_status_error`, `Error::transport`,
  `Error::no_release_found`, ...).
- docs.rs feature badges (`doc(cfg)`) on `AsyncHttpClient`, `AsyncHttpResponse`, and the
  `ReqwestClient` / `ReqwestAsyncClient` / `UreqClient` re-exports.
- The `MoveAll` doc example staged its sources in `/tmp`, which fails with a cross-device error
  since `commit()` renames; it now stages next to the destinations.
- The features docs note that a client without a TLS feature only supports plain-`http` URLs.
- The updater `is_update_available` / `is_update_available_async` docs note that the returned
  release is the newest available, which is not necessarily the release `update()` installs (the
  pipeline prefers the newest semver-compatible one).
- The github and gitea `Update` builders set their auth scheme explicitly instead of relying on
  `AuthScheme::default()` (no behavior change; removes a fragile implicit default).
- The `HttpClient` / `HttpResponse` / `AsyncHttpClient` / `AsyncHttpResponse` docs state the
  trait-evolution policy `ReleaseSource` already had: new methods are only added in minor
  releases with a default implementation, so custom transports keep compiling.

## [1.0.0-rc.5]
Final polish from a full-surface review of rc.4: two breaking `Error`-constructor changes (folded
into the [1.0 migration guide](docs/migrations/0.x-to-1.0-human.md)), additive constructors and
re-exports, and fixes for an s3 url bug, an ungated verify message, and stale docs.

### Added
- `Error::checksum_mismatch(expected, computed)`: build the `ChecksumMismatch` variant, which
  became `#[non_exhaustive]` in rc.4 and had no public construction path.
- `Error::no_release_found_for_target(target)`: the asset-scoped sibling of `no_release_found()`
  (see the constructor change below).
- The `futures_util` and `bytes` crates are re-exported at the root under the `async` feature:
  their types appear in the `AsyncHttpClient`/`AsyncHttpResponse` signatures (`BoxFuture`,
  `BoxStream`, `Bytes`), so a custom async transport no longer needs them as direct dependencies.
- docs.rs builds with the `ureq` feature, so `ureq_agent`, `UreqClient`, and the `ureq` re-export
  now appear in the rendered API docs.

### Changed
- `Error::no_release_found(target: Option<String>)` is split into `no_release_found()` (no
  argument) and `no_release_found_for_target(impl Into<String>)`. Migration:
  `no_release_found(None)` -> `no_release_found()`; `no_release_found(Some(t))` ->
  `no_release_found_for_target(t)`.
- `Error::missing_asset_field` takes `impl Into<String>` (was `&'static str`) and the
  `MissingAssetField` variant's `field` is a `String`, so a custom source can report a dynamic
  field path (e.g. `format!("assets[{i}].url")`). Migration: construction sites are
  source-compatible; a `field` binding from a pattern match is now a `&String`.

### Fixed
- `verify_signature`'s "Verifying downloaded file..." message respects `show_output(false)`; it
  was printed unconditionally.
- An s3 `Endpoint::Generic` URL without a trailing slash is normalized at URL-build time; it
  previously produced malformed (and, under `s3-auth`, wrongly signed) download URLs by
  concatenating the key directly onto the endpoint.
- The github backend percent-encodes `repo_owner`/`repo_name` in API URLs, matching gitlab/gitea
  (no wire change for valid github.com names).
- A header-build failure while following a github/gitlab pagination `Link` propagates as an error
  instead of panicking (matching gitea).
- Doc fixes: the `github`/`gitea` example run commands referenced the removed
  `compression-flate2` feature (and gitea's omitted its required `gitea` feature); the github
  example's Enterprise comment used the renamed-away `.url(...)` setter; both migration guides
  claimed MSRV 1.85 (it is 1.88) and that `is_update_available_async()` was removed (it ships);
  the rc.2 changelog entry claimed default-feature builds compile on 1.85.

## [1.0.0-rc.4]
API polish from a pre-1.0 consumer-experience review: three breaking changes (all folded into the
[1.0 migration guide](docs/migrations/0.x-to-1.0-human.md)) plus additive constructors, inherent
async verbs on the custom backend, and doc fixes.

### Added
- `Error::verification_rejected(reason)`: build the rejection a `verify_binary` hook returns. The
  update pipeline surfaces an already-`VerificationRejected` hook error as-is instead of re-wrapping
  it (other hook errors are still wrapped with their message as the reason).
- The custom backend's `AsyncUpdate<S>` exposes the `*_async` verbs (`update_async`,
  `update_extended_async`, `get_latest_release_async`, `get_newer_releases_async`,
  `get_release_version_async`) as inherent methods, matching the built-in backends' `AsyncUpdate`
  types; `use self_update::AsyncReleaseUpdate` is no longer needed to drive a custom async update.

### Changed
- `ReleaseSource::get_latest_releases` / `AsyncReleaseSource::get_latest_releases` renamed
  `get_releases`: it returns the source's full unfiltered candidate list (newest-first), which the
  old name misstated. Migration: rename the method in `ReleaseSource`/`AsyncReleaseSource` impls;
  the updater's `get_newer_releases` (the filtered fetch) is unchanged.
- The `ChecksumMismatch` and `NotFound` error variants are `#[non_exhaustive]`, matching every
  other struct variant on `Error`. Migration: add `..` to struct patterns
  (`Error::NotFound { url, .. }`); construct a 404 via `Error::http_status_error(404, url)`.
- The single-release endpoints (github `/releases/latest` and `/releases/tags/{ver}`, gitlab/gitea
  by-tag) surface an unparseable response body as `Error::InvalidResponse`, matching the listing
  endpoints (previously `Error::Json`, so detecting "unparseable backend response" required
  matching two variants). Migration: match `Error::InvalidResponse` where `Error::Json` was
  matched on `get_latest_release`/`get_release_version` failures.

### Fixed
- Doc fixes: `Releases::from_listing` documented the pre-rc.3 `Error::MissingField` (now
  `Error::NoCurrentVersion`); two `update.rs` trait docs still described `build()` as returning
  `Box<dyn ReleaseUpdate>` (concrete `Update` since rc.2); the `ureq` feature bullet implied
  reqwest/ureq are mutually exclusive; the migration-guide `match` examples destructured
  `#[non_exhaustive]` variants without `..`.

## [1.0.0-rc.3]
Further 1.0 surface changes on top of rc.2: two API breaks (closing the async-blocking footgun and
aligning the signature-key accessor name), plus correctness, security-hardening, and doc fixes. The
breaks are folded into the [1.0 migration guide](docs/migrations/0.x-to-1.0-human.md).

### Added
- `is_update_available_async()` on every backend's async updater, the async sibling of
  `is_update_available()`.
- `max_download_size(bytes)` on `Download`: an optional cap that aborts a download whose body
  exceeds it (default: no cap).
- `Error::NoCurrentVersion`, returned by `Releases::is_update_available()` on a bare listing with no
  current version (previously a misleading `MissingField { field: "current_version" }`).

### Changed
- `build_async()` returns a distinct `AsyncUpdate` wrapper per built-in backend
  (`github::AsyncUpdate`, `gitlab::AsyncUpdate`, ...) instead of the same `Update` that `build()`
  returns. The wrapper exposes only the async (`*_async`) verbs as inherent methods (no trait import
  needed), so a blocking call such as `.update()` on an async-built updater is now a compile error
  instead of silently blocking the executor. Migration: call the `*_async` verbs and drop any
  `use self_update::AsyncReleaseUpdate`; if you named the return type, use the backend's `AsyncUpdate`.
- Renamed the signature-key accessor `verify_keys()` -> `verifying_keys()`, matching the
  `verifying_keys(...)` setter (`signatures` feature). Migration: rename accessor call sites; the
  setter is unchanged.
- `Releases::is_update_available()` on a bare listing now returns `Error::NoCurrentVersion` instead
  of `Error::MissingField { field: "current_version" }`.
- A user-supplied `Authorization` header (set via `request_header`) is now host-gated like the
  derived auth token: it is not forwarded to a server-chosen next-page or download host unless that
  host is authorized (`allow_auth_host` / a matching origin).
- Zip extraction masks the setuid/setgid/sticky bits from archive entry modes (`mode & 0o777`), so
  an archived `0o4755` entry no longer installs setuid.

### Fixed
- s3 `.release_tag("v1.2.3")` (v-prefixed) now matches; it previously always failed with
  `NoReleaseFound` because stored s3 versions are bare semver.
- The custom backend's `get_newer_releases()` now filters to strictly-newer releases, matching the
  other backends and the trait contract (it previously returned the source's full list).
- GCS bucket listings now paginate past the first page (the listing URL was missing `list-type=2`,
  so continuation tokens were never emitted and later releases were dropped).
- `version::bump_is_compatible` no longer reports a pre-release-to-older comparison (e.g.
  `2.0.5-alpha.0` vs `2.0.3`) as compatible.
- Presigned s3 URLs are redacted in the retry warn-logs (they were previously emitted with a live
  `X-Amz-Signature`).
- Listing response bodies are size-bounded before buffering, and `..` / path separators are
  rejected in a templated `bin_path_in_archive` before extraction.

## [1.0.0-rc.2]
Further breaking changes finalizing the 1.0 surface (still in release-candidate). They make the
HTTP transport an injectable object-safe trait, restructure the error type, finalize the release
model, reshape the feature surface, and change several builder signatures. The crate moves to
edition 2024 and raises the MSRV to 1.88 (the `zip` 8 dependency requires it).

> These changes are folded into the [1.0 migration guide](docs/migrations/0.x-to-1.0-human.md)
> (and its [agent-oriented version](docs/migrations/0.x-to-1.0.md) for automated tooling).

### Added
- `http_client(Arc<dyn HttpClient>)` (and `http_client_async(Arc<dyn AsyncHttpClient>)` under
  `async`) on the `Update`/`ReleaseList` builders: inject any object-safe HTTP client (a test
  double, a wrapper, your application's client), not just reqwest/ureq. The existing
  `reqwest_client` / `ureq_agent` / `reqwest_async_client` setters are now thin wrappers over it.
- Object-safe `HttpClient`/`HttpResponse` traits and async siblings `AsyncHttpClient`/
  `AsyncHttpResponse`, in the now-public `self_update::http_client` module; `reqwest` and `ureq`
  are built-in impls. Implement them for a custom transport. A custom `HttpResponse` implements
  `headers()` + `body()` (the crate parses JSON/XML from the body reader itself).
- A public sealed `AsyncReleaseUpdate` trait mirroring `ReleaseUpdate` for the async verbs
  (re-exported at the crate root under `async`), usable as a bound.
- `ReleaseList::fetch_async` on every backend (under `async`), the async sibling of `fetch`.
- `is_update_available() -> Result<Option<Release>>` on every backend's `Update`, returning the
  newest strictly-newer release (or `None` when up to date).
- `self_update::Certificate` (an opaque PEM/DER root CA) plus `add_root_certificate` on the
  `Update`/`ReleaseList` builders and on `Download`, so a private/internal CA can be trusted
  without injecting a whole pre-built client. A malformed certificate surfaces as
  `Error::InvalidCertificate` from `build()` / `download_to`.
- `allow_auth_host(host)` on the builders, to authorize an asset CDN/mirror host to receive the
  auth token, and `dangerously_allow_non_https_auth_forwarding()` to allow the token over http to a
  host-matched request (see the auth-token origin change below).
- Public `Error` constructors for custom `ReleaseSource` implementors: `Error::no_release_found`,
  `missing_asset_field`, `invalid_response`, `http_status_error` (the release-flow variants are
  `#[non_exhaustive]`, so these are the way to build them downstream).
- `Error::CompressionNotEnabled`, returned when a gzip asset is detected but `compression-tar-gz`
  is off (previously a plain `.gz` installed its still-compressed bytes as the binary).
- `progress-bar` feature (default-on) gating `indicatif` and the terminal bar. The byte-level
  `progress_callback` and `show_download_progress` stay always-on; with the feature off the bar
  is a no-op.
- Per-backend features: `github` (default), `gitlab`, `gitea`, `s3` (each gates its
  `backends::<name>` module); `s3` also gates `quick-xml`.
- `ProgressStyle` type (template + chars), passed to the `progress_style` setter so the two
  strings can't be transposed (under `progress-bar`).
- `version::cmp_versions(a, b) -> Result<Ordering>`, a total-order semver comparison. (`version::cmp_releases_newest_first` is crate-internal and not part of the public API.)
- `ReleaseStatus::version() -> Option<&str>`, mirroring `VersionStatus::version` (`None` on the
  up-to-date arm).
- `Releases::from_releases(..)` and `Releases::from_listing(..)` for constructing a `Releases` in
  downstream tests (the latter is the bare-listing state with no current version).
- s3 `max_keys` (a `u16`, clamped `1..=1000`, default 1000) and `signature_ttl` (`s3-auth`,
  clamped to AWS's `X-Amz-Expires` range of 1s..=7d, default 300s) setters, replacing the
  hardcoded 100-key cap and 300s presigned-URL TTL.
- `retry_backoff(base, max)` on the `Update`/`ReleaseList` builders to configure the exponential
  retry backoff (default 100ms base, ~3.2s cap).

### Changed
- Edition 2024. MSRV raised from 1.85 to 1.88 (required by the `zip` 8 dependency and by
  1.88 language features used unconditionally in the crate).
- Default features changed from `["reqwest", "default-tls"]` to
  `["reqwest", "rustls", "progress-bar", "github", "archive-tar", "compression-tar-gz"]`: default
  TLS is now `rustls`, only the `github` backend is on by default (gitlab/gitea/s3 opt-in), and the
  tar/gzip archive support most releases need is on by default (zip stays opt-in).
- Renamed features: `default-tls` -> `native-tls`, `compression-flate2` -> `compression-tar-gz`.
  `s3-auth` now implies `s3`.
- `reqwest` + `ureq` and `native-tls` + `rustls` are no longer mutually exclusive: the
  compile-time guards are removed (the transport is now a runtime trait seam). The sync API
  prefers `reqwest` when both clients are on; the per-call builders prefer `rustls` when both TLS
  features are on. `cargo build --all-features` builds. The only remaining guards are
  "at least one client" and "`async` requires `reqwest`".
- `build()` returns the concrete backend `Update` instead of `Box<dyn ReleaseUpdate>`. `Update` is
  `Send` (so it can move to a worker thread) and exposes `update`, `update_extended`,
  `get_latest_release`, `get_newer_releases`, `get_release_version`, and `is_update_available` as
  inherent methods, so `.build()?.update()?` needs no trait import.
- The `Error` type has no stringly-typed catch-all. `Error::Config(String)` is removed and split
  into `Error::MissingField { field }`, `Error::InvalidHeader { source }`,
  `Error::InvalidAuthToken { source }`, `Error::InvalidCertificate { source }` (root-certificate /
  client-build failures), `Error::InvalidProgressStyle { source }` (a bad progress-bar template,
  under `progress-bar`), and the s3-auth SigV4 host-extraction case now maps to `Error::S3Auth`.
- `Error::Release(String)` is split into `Error::NoReleaseFound { target }`,
  `Error::MissingAssetField { field }`, and `Error::InvalidResponse { source }`. A malformed
  (non-array) release-listing body now maps to `InvalidResponse`, not `NoReleaseFound`.
- `Error::Update(String)` is split into `Error::VerificationRejected { reason }` and
  `Error::Internal { message, source }`. `ChecksumMismatch` / `Aborted` are unchanged.
- `Error::source()` now chains the wrapped cause for `InvalidResponse`, `InvalidHeader`,
  `InvalidAuthToken`, `InvalidCertificate`, `InvalidProgressStyle`, and `Internal` (when it wraps
  one). The new struct variants and `Unauthorized`/`HttpStatus` are `#[non_exhaustive]`
  (destructure with `..`). `Error::Io` still wraps a concrete `std::io::Error`.
- The sync `HttpResponse` trait requires only `headers()` + `body()` (plus the defaulted
  `body_buffered()`); the `&mut self` `json_value` / `text` methods are removed.
- `Release` and `ReleaseAsset` fields are now private (`pub(crate)` `Arc<str>`); read them through
  borrow getters of the same name (`name()`, `version()`, `date()`, `body() -> Option<&str>`,
  `assets()`, and `name()` / `download_url()` on the asset). Construction stays via
  `Release::builder()` / `ReleaseAsset::new(..)`.
- `ReleaseList::fetch` returns `Result<Releases>` instead of `Result<Vec<Release>>`; recover the
  vector with `.into_vec()`. A bare listing has `current_version() == None`.
- The `ReleaseUpdate` filtered fetch is renamed `get_latest_releases` -> `get_newer_releases`
  (and `AsyncReleaseUpdate`'s to `get_newer_releases_async`); it returns only releases strictly
  newer than the current version. The raw `get_latest_release` (singular) is unchanged. The
  custom-source `ReleaseSource::get_latest_releases` keeps its name (it returns the source's
  candidate list, unfiltered) and no longer takes a `current_version` argument.
- The github endpoint setter is renamed `url` -> `api_base_url` (it takes the full API base,
  including any `/api/v3` path); the gitlab and gitea instance-base setter is renamed `url` ->
  `host`.
- The signature-key setter is renamed `verify_keys` -> `verifying_keys`.
- The post-update verify hook is renamed `verify_with` -> `verify_binary` and its callback returns
  `Result<()>` instead of `bool` (`Err(..)` rejects, producing `Error::VerificationRejected`).
- `progress_style` takes a typed `ProgressStyle` instead of two `impl Into<String>` args (under
  `progress-bar`).
- `Download::header` is renamed `Download::request_header`, and it is now infallible (returns
  `&mut Self`); an invalid header is deferred and surfaced from `download_to` as
  `Error::InvalidHeader`, matching the builders.
- `Download::from_url` takes `impl Into<String>`; `Extract::from_source` / `extract_into` /
  `extract_file`, `Move::from_source` / `replace_using_temp` / `to_dest`, and `MoveAll::from_temp`
  take `impl AsRef<Path>`, and `Extract` / `Move` / `MoveAll` dropped their lifetime parameter.
- s3 `EndPoint` is renamed `Endpoint`, its `Generic` variant is now a tuple `Generic(String)`, and
  the `end_point(..)` setter is renamed `endpoint(..)`.
- The `auth_token` is applied to both the release-listing and the binary-download requests, but
  only to requests whose host matches the backend's configured API host (or an `allow_auth_host`
  entry), over https. A server-supplied asset `download_url` or pagination `Link` pointing at a
  different host does not receive the token. The scheme is per backend (`token` for github/gitea,
  `Bearer` for gitlab) and a user-set `Authorization` via `request_header` overrides it.
- During `update()`, releases on each page that are not strictly newer than the current version
  are filtered out per-item; pagination continues through all pages regardless. `ReleaseList::fetch`
  walks every page unfiltered. The s3 listing follows `NextContinuationToken` so multi-page
  buckets list fully.
- The `retries` budget now also covers the binary download's request-establishment phase (before
  any bytes stream); a mid-stream failure is still not retried.
- `Move::to_dest` falls back to copy-and-rename when source and destination are on different
  filesystems, instead of failing with EXDEV. Zip extraction rejects entries that would escape the
  output directory and preserves unix permission modes. The interactive confirmation prompt treats
  a closed stdin (EOF) as a decline. s3 object keys are percent-encoded exactly once in the SigV4
  canonical URI and the signed URL. s3 presigned-URL secrets are redacted from error messages.

### Removed
- `Error::Config(String)` (see the structured variants above).
- `HttpResponse::json_value` and `HttpResponse::text` (a custom transport implements only
  `headers()` + `body()`).
- The deprecated no-op s3 `auth_token` setter (use `.access_key((id, secret))` under `s3-auth`).
  `auth_token` remains functional on github/gitlab/gitea.
- `Download::reqwest_client`, `Download::reqwest_async_client`, and `Download::ureq_agent`:
  configure a custom client on the `Update` builder (which forwards it to the download) instead.
- The `reqwest`/`ureq` and `native-tls`/`rustls` mutual-exclusion `compile_error!` guards (the
  clients and TLS backends now coexist).

### Migration guide
The full walkthrough is in [`docs/migrations/0.x-to-1.0-human.md`](docs/migrations/0.x-to-1.0-human.md)
(and an [agent-oriented version](docs/migrations/0.x-to-1.0.md) for automated tooling).

## [1.0.0-rc.1]
First release candidate for 1.0. This version makes a number of breaking changes to clean up the
public API surface before committing to long-term stability. Future `1.x` releases will remain
backwards compatible.

> **Upgrading from 0.x?** See the [1.0 migration guide](docs/migrations/0.x-to-1.0-human.md)
> for a complete walkthrough of every breaking change (and an
> [agent-oriented version](docs/migrations/0.x-to-1.0.md) for automated tooling).

### Added
- S3 auth support (request signing for private buckets) behind the `s3-auth` feature
  ([#172](https://github.com/jaemk/self_update/pull/172)).
- Re-export the `http` crate as `self_update::http`, so consumers can name the header
  types accepted by `Download::header`/`replace_headers` (e.g.
  `self_update::http::header::ACCEPT`) without a separate `http` dependency.
- Re-export `ReleaseUpdate`, `ReleaseStatus`, `Release`, and `ReleaseAsset` at the crate root
  (e.g. `self_update::ReleaseUpdate`) - the types returned by `update_extended()` / `fetch()`.
- Re-export `zipsign_api` and add a `self_update::VerifyingKey` type alias (under the
  `signatures` feature) so `verify_keys(...)` callers need neither a direct
  `zipsign-api` dependency nor a hard-coded key length.
- `asset_identifier(...)` builder setter on the `gitlab` and `s3` `UpdateBuilder`s (it already
  existed on `github`/`gitea`), so every backend can disambiguate multiple matching assets.
- `compile_error!` guards that turn invalid feature combinations into a clear message:
  enabling both or neither of `reqwest`/`ureq`, or both `default-tls`/`rustls`.
- `#[must_use]` on every builder type.
- A new `Releases` type, returned by the release-fetch methods, carrying the fetched releases plus
  the updater's current version. It has `all() -> &[Release]`, `latest() -> Option<&Release>`
  (newest first), `into_vec() -> Vec<Release>`, `len()`/`is_empty()`, `current_version() -> &str`,
  `IntoIterator` (owned and borrowed), and `is_update_available() -> Result<bool>` (true when the
  latest release is strictly newer than the current version). The light pre-check is
  `updater.get_latest_releases()?.is_update_available()` (sync) or
  `updater.get_latest_releases_async().await?.is_update_available()` (async), which fetches the
  release list once instead of fetching twice. `Releases` is re-exported at the crate root and is
  distinct from the per-backend `ReleaseList` builder.
- Documented paths for each backend's per-backend `ReleaseList` type
  (`self_update::backends::<github|gitlab|gitea|s3>::ReleaseList`); the four backends each have a
  distinct `ReleaseList` builder, so they are surfaced consistently rather than unified under one
  crate-root type.
- A new `examples/custom.rs` showing the custom backend: a minimal `ReleaseSource` impl driving a
  sync update, plus an async variant under the `async` feature.
- `ReleaseStatus::updated_release() -> Option<&Release>` and `into_updated_release() -> Option<Release>`
  to read the installed release without a `match` (which `#[non_exhaustive]` would force a wildcard
  arm onto).
- `Error::http_status() -> Option<u16>`, returning the HTTP status for a completed non-2xx response
  (`NotFound` => 404, `Unauthorized`/`HttpStatus` => their code) and `None` otherwise.
- `Error::url() -> Option<&str>`, the failing request URL for `NotFound`/`Unauthorized`/`HttpStatus`
  and `None` otherwise, mirroring `http_status()`.
- `Error::ChecksumMismatch { expected, computed }` (a checksum digest mismatch) and `Error::Aborted`
  (the user declined the interactive confirmation prompt). Both were previously folded into the
  catch-all `Error::Update`. The fields/variant let a caller branch on these outcomes instead of
  matching a string.
- `unattended()` on every backend `Update` and `ReleaseList` builder: sets `no_confirm(true)` and
  `show_output(false)` in one call for daemon/CI use. The default `no_confirm == false` blocks on
  stdin for an interactive confirmation.
- `backends::s3::AccessKey::new(access_key_id, secret_access_key)`, a named constructor alongside the
  existing `From` conversions.
- `Display` for `ArchiveKind`, rendering a human-readable name (`tar.gz`, `zip`, ...) used in error
  messages instead of the `Debug` form.
- docs.rs now renders feature-gate badges (`#[doc(cfg(...))]` on the gated re-exports, built with
  `rustdoc-args = ["--cfg", "docsrs"]`), and the crate-level docs open with a Quick start example.
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
  .verify_checksum(Checksum::Sha256(hex))` (or `Checksum::Sha512(..)`) verifies the
  downloaded artifact against a known digest - e.g. one published in a `SHA256SUMS` file -
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
  moves atomically - either all succeed, or on the first failure every applied move is rolled back,
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
  `update_extended_async()`, `get_latest_release_async()` / `get_latest_releases_async()` (returning
  `Result<Releases>`), and `get_release_version_async()` (returning `Result<Release>`). The blocking
  API is unchanged and the async path reuses the same response parsers and the same extract/install
  tail (no logic fork); only the release listing and the download are async. It is tokio-only and
  reqwest-only (`async` is incompatible with `ureq`).
- Custom backends: a new public `ReleaseSource` trait (three fetch methods - `get_latest_release`,
  `get_latest_releases`, `get_release_version`) plus a `backends::custom::Update` builder let you
  update from a host the built-in backends don't cover (another forge, a private registry, a plain
  HTTP directory). You implement only *where releases come from*; the crate runs its usual
  compare -> select-asset -> download -> verify -> extract -> install flow over your source. The
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
    builder - each now matching its `ReleaseUpdate` accessor of the same name;
  - the verification setters are `verify_checksum(...)` (was `verifying_checksum`) and
    `verify_keys(...)` (was `verifying_keys`), matching their `verify_checksum()` / `verify_keys()`
    accessors;
  - the `Update`/`Download` progress setters are `progress_callback(...)` and `progress_style(...)`
    (were `set_progress_callback`/`set_progress_style`).
- **`Download` setters renamed to match the `Update`/`ReleaseList` builders**: `set_timeout` ->
  `timeout`, `set_header` -> `header`, `set_headers` -> `replace_headers` (it replaces the whole
  `HeaderMap`), `show_progress` -> `show_download_progress`, `set_progress_callback` ->
  `progress_callback`, `set_progress_style` -> `progress_style`. The old names are gone, not even
  available as `#[doc(alias)]`s; use the canonical method name.
- **`request_header(name, value)` now accepts `TryInto<HeaderName>`/`TryInto<HeaderValue>`** on the
  `Update` and `ReleaseList` builders, so `.request_header("X-Foo", "bar")` works (no
  `.parse().unwrap()`); an invalid header is surfaced as `Error::Config` from `build()` instead of
  panicking. Typed-argument call sites still compile.
- **`ReleaseUpdate` is now a sealed trait** - downstream code can call it (every backend's
  `build()` returns a `Box<dyn ReleaseUpdate>`) but can no longer implement it for foreign
  types.
- **`ReleaseUpdate` accessors return borrows**: `current_version`/`target`/`bin_name`/
  `bin_path_in_archive`/`progress_template`/`progress_chars` return `&str`, and
  `release_tag`/`asset_identifier`/`auth_token` return `Option<&str>` (were owned `String`/
  `Option<String>`); `api_headers` takes `Option<&str>`.
- **`ReleaseUpdate` accessors renamed to match their setters**: the `target_version` accessor is
  now `release_tag` and `identifier` is now `asset_identifier`.
- **The release-fetch surface on the sealed `ReleaseUpdate` trait now returns the new `Releases`
  type**: `get_latest_release()` and `get_latest_releases()` return `Result<Releases>` (the latter no
  longer takes a `current_version` argument).
- **`ReleaseSource` is sync-only; a separate `AsyncReleaseSource` trait drives the async custom
  updater.** `ReleaseSource` is the three sync fetch methods plus `Send + Sync` (no `Clone` bound).
  For a natively-async source, implement the new public `AsyncReleaseSource` trait (the same three
  fetches as `async fn`, clean names without an `_async` suffix) and drive it through
  `backends::custom::AsyncUpdate` + `build_async()`; to reuse a `Clone` sync `ReleaseSource` from the
  async API, wrap it in `backends::custom::Blocking` (which runs the sync fetches on
  `tokio::task::spawn_blocking`). `AsyncReleaseSource` is consumed through generics (`AsyncUpdate<S>`,
  never a `dyn` object), so its `async fn`s need no `async-trait`/boxing. The trait also enforces
  `Send` on its returned futures at the type level, so a non-`Send` impl fails to compile at the impl
  site rather than later at the spawn site. See *Added* below.
- **`ReleaseUpdate` accessors moved to a sealed `UpdateConfig` supertrait** (`ReleaseUpdate:
  UpdateConfig`). All the getters (`current_version`, `target`, `bin_name`, `release_tag`,
  `asset_identifier`, `auth_token`, `api_headers`, the progress/transport/verify getters, ...) now
  live on `self_update::UpdateConfig`; `ReleaseUpdate` keeps the fetches plus
  `update`/`update_extended`. Calling an accessor on a `Box<dyn ReleaseUpdate>` is unchanged; a
  generic helper bounded `R: ReleaseUpdate` that calls an accessor needs `use self_update::UpdateConfig;`.
  The accessor `bin_install_path()` returns `&Path` (was an owned `PathBuf`); only relevant if you
  named the return type.
- **`#[derive(Clone)]` added to every `UpdateBuilder`** (github/gitlab/gitea/s3/custom), matching
  the already-`Clone` `ReleaseListBuilder`s, so a configured builder can be cloned before `build()`.
- **`ReleaseList::fetch` now takes `&self`** (was `self`) on the `github`/`gitlab`/`gitea`
  backends (the `s3` backend already borrowed).
- **`Error` is now `#[non_exhaustive]`**, and the feature-specific variants are restructured:
  the old `Reqwest`/`Ureq` variants become a single opaque `Error::Transport` (a request that could
  not complete: connection, TLS, timeout), and a completed non-2xx response - previously
  `Error::Network(String)` - is now one of `Error::NotFound { url }` (404),
  `Error::Unauthorized { status, url }` (401/403), or `Error::HttpStatus { status, url }` (any other
  non-2xx), so a consumer can distinguish release-not-found from auth failure from other statuses.
  Both the `reqwest` and `ureq` clients now produce the same status variants. Inspect them with
  `Error::http_status()` / `Error::url()`. The `StdTimeError`/`TimeError`/`Digest`/`UrlParse`
  (`s3-auth`) variants are collapsed into a single opaque `Error::S3Auth`. The underlying error is
  still reachable via `Error::source()`.
- `Error::Zip`, `Error::Signature`, `Error::Json`, and `Error::SemVer` are now opaque: each wraps
  `Box<dyn std::error::Error + Send + Sync>` instead of the concrete `zip::result::ZipError` /
  `zipsign_api::ZipsignError` / `serde_json::Error` / `semver::Error`. Code that matched the inner
  dependency type must inspect it via `Error::source()` (or downcast the box) instead.
- `Error::NonUTF8` is renamed to `Error::SignatureNonUTF8` (`signatures` feature).
- A checksum digest mismatch is now `Error::ChecksumMismatch { expected, computed }` and a declined
  confirmation prompt is now `Error::Aborted`; both were previously folded into `Error::Update`. Code
  that matched `Error::Update` for these cases must switch to the new variants. Genuine internal
  failures (blocking-task join, extractor invariants, verify-callback rejection) stay `Error::Update`.
- `Error` Display strings are normalized: `ArchiveNotEnabled` now renders with the
  `"ArchiveNotEnabledError: ..."` prefix and `SignatureNonUTF8` with `"SignatureError: ..."`,
  matching the `<Name>Error:` prefix of every other variant. Display strings are human-facing and
  may change between releases; match on variants or use `http_status()` / `url()` for programmatic
  decisions.
- The two update-result enums are renamed: `Status` (the lightweight result of `update()`, carrying
  a version string) is now `VersionStatus`, and `UpdateStatus` (the extended result of
  `update_extended()`, carrying a `Release`) is now `ReleaseStatus`. The method
  `UpdateStatus::into_status(current_version)` is now `ReleaseStatus::into_version_status(current_version)`.
  Both are re-exported at the crate root and `#[non_exhaustive]`. The status predicates are renamed
  to a matching pair: `uptodate()` is now `is_up_to_date()` and `updated()` is now `is_updated()`, on
  both `VersionStatus` and `ReleaseStatus`.
- `ArchiveKind`, `Compression`, `Release`, and `ReleaseAsset` are now `#[non_exhaustive]`, as are
  `Download`, `Extract`, `Move`, and `MoveAll`, and each backend's concrete `Update` and
  `custom::AsyncUpdate` structs (the return types of `build_async()`).
- The string builder setters now take `impl Into<String>` instead of `&str`:
  `current_version`, `release_tag`, `target`, `asset_identifier`, `bin_name`, `bin_path_in_archive`,
  `auth_token`, and the backend setters `repo_owner` / `repo_name` / `url` / `filter_target` /
  `bucket_name` / `asset_prefix` / `region`. String-literal call sites are unchanged; a site that
  passed `&some_string` now passes the `String` itself (drop the `&`) or `some_string.clone()`.
- The s3 `Update` and `ReleaseList` `build()` now validate the endpoint/region pairing: the `S3`,
  `S3DualStack`, and `DigitalOceanSpaces` endpoints require a `region`, so a missing region is an
  `Error::Config` from `build()` rather than from the first request. `GCS` and `Generic` endpoints
  are unaffected.
- `bin_name` re-derives `bin_path_in_archive` when that path was auto-derived, so calling `bin_name`
  twice no longer leaves a stale archive path. An explicitly-set `bin_path_in_archive` stays sticky.
- `Download::header()` now takes `TryInto<HeaderName>` / `TryInto<HeaderValue>` and returns a
  `Result`, so string literals work (`.header("Accept", "application/octet-stream")?`); an invalid
  header is reported instead of requiring a pre-parsed `HeaderName`/`HeaderValue`.
- `Download::progress_style` and each backend's `UpdateBuilder::progress_style` now
  accept `impl Into<String>`.
- The `Download::header` doc example now uses `self_update::http::header::ACCEPT`, so it
  is client-agnostic and self-contained.
- `backends::custom::Blocking`'s inner field is now private. Construct it with
  `Blocking::new(source)` and read the wrapped source via `into_inner()` / `as_inner()`.
- `DEFAULT_PROGRESS_TEMPLATE` and `DEFAULT_PROGRESS_CHARS` are no longer public (internal defaults
  only).
- The `verify_keys(...)` setter and accessor now use the `self_update::VerifyingKey` alias in
  their signatures (instead of the raw `[u8; zipsign_api::PUBLIC_KEY_LENGTH]` array).
- The s3 `AccessKey` credential type is now public and re-exported as
  `self_update::backends::s3::AccessKey` (under `s3-auth`), and is `#[non_exhaustive]` so a future
  credential field (e.g. an STS session token) can be added without a break. Build it via its
  `(id, secret)` `From` impls or `AccessKey::new`.
- A builder `build()` error for a missing required field now names the setter to call, e.g.
  `` `current_version` required (call `.current_version(...)`) `` and `` `bin_name` required (call
  `.bin_name(...)`) ``.
- Respect pagination URLs when fetching GitHub releases
  ([#179](https://github.com/jaemk/self_update/pull/179)).
- Print a short "up to date" message instead of nothing when no update is available
  ([#180](https://github.com/jaemk/self_update/pull/180)).

### Removed
- The per-updater `is_update_available()` / `is_update_available_async()` checks. They fetched the
  release list a second time; fetch once and call `is_update_available()` on the returned `Releases`
  instead (`updater.get_latest_releases()?.is_update_available()`).
- The s3 `UpdateBuilder::auth_token` setter is now a `#[deprecated]` no-op shim that points at
  `.access_key((id, secret))` (the `s3-auth` feature). The S3 backend authenticates by signing
  requests with `access_key` (AWS SigV4), never a bearer token, so the setter never had any effect
  there; the shim stores nothing and exists only so a config ported from a git backend self-diagnoses
  with a deprecation hint instead of a "no method" error. `auth_token` remains functional on
  github/gitlab/gitea.
- The `Error::Reqwest`, `Error::Ureq`, and `Error::Network` variants (now `Error::Transport` for an
  incomplete request and `Error::NotFound`/`Unauthorized`/`HttpStatus` for a completed non-2xx
  response), and the `Error::StdTimeError`, `Error::TimeError`, `Error::Digest`, and `Error::UrlParse`
  variants (replaced by `Error::S3Auth`).
- The `self_replace` and `tempfile::TempDir` re-exports (`self_update::self_replace` /
  `self_update::TempDir`). They pinned consumers to the crate's exact dependency versions; depend
  on `self-replace` / `tempfile` directly instead. The `http`, `reqwest`/`ureq`, and `zipsign_api`
  re-exports are unchanged.
- `GetArchiveReaderResult` is no longer `pub` (it leaked `either::Either` /
  `flate2::read::GzDecoder` for a private function and had no consumer use).
- The deprecated `std::error::Error::description` implementation on `Error`.
- `should_update` (deprecated since 0.4.2) - use `version::bump_is_greater` or
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
  - `.with_url(` -> `.url(`  (github API endpoint, kept)
  - `.with_host(` -> `.url(` (gitlab, gitea)
- Release-list target filter: `.with_target(` -> `.filter_target(`
- S3 credentials: `.access_key_id(` -> `.access_key(`
- Version tag / asset id setters: `.target_version_tag(` -> `.release_tag(`, `.identifier(` ->
  `.asset_identifier(`
- Verification setters: `.verifying_checksum(` -> `.verify_checksum(`, `.verifying_keys(` ->
  `.verify_keys(`
- `Download`/`Update` progress + transport setters: `.set_progress_callback(` ->
  `.progress_callback(`, `.set_progress_style(` -> `.progress_style(`, and on `Download`
  `.set_timeout(` -> `.timeout(`, `.set_header(` -> `.header(`, `.set_headers(` ->
  `.replace_headers(`, `.show_progress(` -> `.show_download_progress(`. The old names are gone (not
  even `#[doc(alias)]`s).
- Status enums: `Status` -> `VersionStatus`, `UpdateStatus` -> `ReleaseStatus`,
  `into_status(` -> `into_version_status(`. Status predicates: `.uptodate(` -> `.is_up_to_date(`,
  `.updated(` -> `.is_updated(`.
- Dropped re-exports: replace `self_update::self_replace::` / `self_update::TempDir` with a direct
  `self-replace` / `tempfile` dependency.
- Error matching (the enum is now `#[non_exhaustive]`, so add a `_ =>` arm):
  - `Error::Reqwest(e)` / `Error::Ureq(e)` -> `Error::Transport(e)`
  - a completed non-2xx response was `Error::Network(_)`; it is now one of
    `Error::NotFound { url }` (404), `Error::Unauthorized { status, url }` (401/403), or
    `Error::HttpStatus { status, url }` (any other non-2xx). Call `Error::http_status()` /
    `Error::url()` to inspect them.
  - `Error::StdTimeError(e)` / `Error::TimeError(e)` / `Error::Digest(e)` / `Error::UrlParse(e)` -> `Error::S3Auth(e)`
- `ReleaseUpdate` accessors now return borrows: append `.to_string()` where you previously
  got an owned `String` (e.g. `updater.current_version().to_string()`).
- Custom `impl ReleaseUpdate for MyType` is no longer possible (the trait is sealed); implement the
  public `ReleaseSource` / `AsyncReleaseSource` trait and drive it through `backends::custom`.
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

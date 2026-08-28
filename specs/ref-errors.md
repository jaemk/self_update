# Error model (reference)

Status: implemented

## Scope

The crate's single public error type `errors::Error` (re-exported as `self_update::errors::Error`),
its `Result<T>` alias, the `Display` / `std::error::Error` (`source()`) impls, the `From`
conversions, the `http_status()` helper, the `url()` accessor, and the public constructors for
custom `ReleaseSource` implementors. Source of truth:
`src/errors.rs`. Construction sites are spread across the backends, the HTTP clients, the update
pipeline, and the checksum module.

## Behavior

`Error` is declared `#[derive(Debug)] #[non_exhaustive] pub enum` at `errors.rs`. Every variant,
what produces it, and its feature gate:

Every struct-form variant is marked `#[non_exhaustive]` (in addition to the enum-level
`#[non_exhaustive]`) so fields can be added without a breaking change. This includes
`NotFound` and `ChecksumMismatch` (aligned with their siblings in 1.0.0-rc.4); downstream
code builds them via the public constructors (`http_status_error(404, ..)`,
`checksum_mismatch(..)`).

| Variant | Produced by | Feature gate | Opaque/boxed? |
| --- | --- | --- | --- |
| `Internal { message: String, source: Option<Box<dyn Error + Send + Sync>> }` | Genuine internal invariants / task failures: extractor source has no file name (`lib.rs`), path not in archive, non-UTF-8 archive path (`lib.rs`), and blocking-task join failure (`custom.rs`, `update.rs`). The join sites carry the tokio `JoinError` as `source`; the invariant sites set `source: None`. `#[non_exhaustive]`. | none | source boxed when present |
| `VerificationRejected { reason: Option<String> }` | The post-update `verify_binary` callback returned `Err(..)`, so nothing was installed (`update.rs`). `reason` carries `Some(<error message>)` from the callback's returned error. `#[non_exhaustive]`. | none | no (struct fields) |
| `ChecksumMismatch { expected: String, computed: String }` | The downloaded artifact's digest did not match the configured `Checksum` (`checksum.rs`). Both fields are lowercase hex-encoded digests. `#[non_exhaustive]`. | none (compiled unconditionally) | no (struct fields) |
| `Aborted` | The user declined the interactive confirmation prompt (`lib.rs` `confirm()`). | none | no (unit) |
| `NotFound { url: String }` | A request completed and returned HTTP 404. Raised by both HTTP clients when the response status is 404. `#[non_exhaustive]`. | none | no (struct fields) |
| `Unauthorized { status: u16, url: String }` | A request completed and returned HTTP 401 or 403. `status` holds the exact code. Raised by both HTTP clients. `#[non_exhaustive]`. | none | no (struct fields) |
| `RateLimited { status: u16, url: String, reset_at: Option<SystemTime>, retry_after: Option<Duration> }` | A 429 (always), or a 403 whose response carried a spent request quota (`x-ratelimit-remaining: 0`, or gitlab's `RateLimit-Remaining: 0`) *or* a usable `Retry-After` (GitHub's secondary rate limit, which can answer 403 + `Retry-After` while the remaining-quota header is still nonzero; a `Retry-After: 0` does not count as usable, see below). Raised by both HTTP clients via `status_to_error_with_headers` (`errors.rs:966-981`), including the ureq injected-agent path (`http_client/ureq.rs:156-176` applies a per-request `http_status_as_error(false)` override, skipped when the injected agent's own config already disables it, so it reaches the header-aware check; the `StatusCode` arm at `ureq.rs:184-198` is now only a defensive fallback). `reset_at` comes from `x-ratelimit-reset` / `RateLimit-Reset` (a unix timestamp), `retry_after` from a delta-seconds `Retry-After`; both `None` when absent, unparseable, or beyond the 24h ceiling (`MAX_RATE_LIMIT_WAIT`, `errors.rs:882`). A `Retry-After: 0` is treated the same as absent (`parse_retry_after` floors a zero delay to `None`), so a bare 403 carrying only `Retry-After: 0` stays `Unauthorized` instead of becoming a zero-wait `RateLimited`. `#[non_exhaustive]`. | none | no (struct fields) |
| `HttpStatus { status: u16, url: String }` | A request completed and returned any other non-2xx status (e.g. 500, 503). Raised by both HTTP clients. `#[non_exhaustive]`. | none | no (struct fields) |
| `NoReleaseFound { target: Option<String> }` | The clean negative of a release lookup: no release / no matching release for a tag/version (`github.rs`, `gitlab.rs`, `gitea.rs`, `s3.rs`), or the resolved release had no asset for the requested target (`update.rs`, with `target: Some(...)`). `#[non_exhaustive]`. | none | no (struct fields) |
| `MissingAssetField { field: String }` | A release/asset payload was missing a required field (`url`/`name`/`tag_name`/`created_at`/`assets`/`browser_download_url`/`assets.links`) in each backend's DTO conversion (`github.rs`, `gitlab.rs`, `gitea.rs`). `String` so a custom source can report a dynamic field path (e.g. `assets[2].url`). `#[non_exhaustive]`. | none | no (struct fields) |
| `InvalidResponse { source: Box<dyn Error + Send + Sync> }` | A backend response could not be parsed: a malformed (non-array) JSON release-listing body (`github.rs`, `gitlab.rs`, `gitea.rs`), the S3 listing regex build failure, and the S3 XML parse failure (`s3.rs`). The underlying error is carried as `source`. `#[non_exhaustive]`. | none | yes (boxed source) |
| `MissingField { field: &'static str }` | A required builder/configuration field was not set: `current_version`/`bin_name`/`bin_path_in_archive` (`common.rs`), `version` (`update.rs`), `source` (`custom.rs`), `repo_owner`/`repo_name` (`github.rs`, `gitlab.rs`, `gitea.rs`), `host` (`gitea.rs`), `bucket_name`/`region` (`s3.rs`). `#[non_exhaustive]`. | none | no (struct fields) |
| `InstallPathNotWritable { path: PathBuf }` | The opt-in preflight probe (`check_install_path_writable(true)`, `probe_writable` at `update.rs:1865`) when the path is definitely not writable, or the install step (`map_install_io_error` at `update.rs:1582`) when the replace/move fails with `PermissionDenied`. `path` is the configured `bin_install_path`, or in bundle mode the bundle's parent directory. `#[non_exhaustive]`. | none | no (struct fields) |
| `NoAppBundle { exe: PathBuf }` | Bundle mode with no explicit `bundle_install_path` on macOS, when `current_exe()` has no `.app` ancestor to derive it from (`default_bundle_install_path` at `update.rs:1801`). `exe` is the running executable. macOS only: other targets get `MissingField { field: "bundle_install_path" }`. `#[non_exhaustive]`. | none | no (struct fields) |
| `ConflictingConfig { field: &'static str, conflict: &'static str }` | Two builder settings that cannot both apply were set; raised from `build()` (`resolve_bundle_mode` at `backends/common.rs:638`) for `bundle_path_in_archive` combined with an explicit `bin_install_path` or `bin_path_in_archive`. `field` is the rejected setting, `conflict` the one it clashes with. `#[non_exhaustive]`. | none | no (struct fields) |
| `AppTranslocated { exe: PathBuf }` | Bundle mode on macOS when the running executable is inside an `AppTranslocation` mount, i.e. a quarantined copy on a read-only randomized path whose bundle is not the installed one (`is_translocated` at `update.rs:1838`, via `default_bundle_install_path`). `#[non_exhaustive]`. | none | no (struct fields) |
| `InvalidHeader { source: Box<dyn Error + Send + Sync> }` | A request header (`request_header` on the builders or on `Download`) was not a valid HTTP header. The setters are infallible; the error is deferred and surfaced from `build()` (via `common.rs`) or from `Download::download_to` / `download_to_async` (`lib.rs`). The source is a crate-internal `MessageError` carrying the validation message. `#[non_exhaustive]`. | none | yes (boxed source) |
| `InvalidAuthToken { source: Box<dyn Error + Send + Sync> }` | An auth token could not be encoded as an HTTP `Authorization` header value (`github.rs`, `gitlab.rs`, `gitea.rs`, `update.rs`). The underlying header-value parse error is carried as `source`. `#[non_exhaustive]`. | none | yes (boxed source) |
| `InvalidCertificate { source: Box<dyn Error + Send + Sync> }` | A custom TLS root certificate could not be parsed, or the HTTP client that would trust it could not be built. Produced by `RequestConfig::check()` (`common.rs`, surfaced from `build()`) and by `Download::download_to` / `download_to_async` (`lib.rs`) when `add_root_certificate` certs are supplied. Exception: on a ureq-only build a malformed **DER** certificate is not caught at `build()` (ureq's `from_der` is infallible) and surfaces as `Transport` at connection time; PEM is validated at `build()` on both clients. `#[non_exhaustive]`. | none | yes (boxed source) |
| `InvalidProgressStyle { source: Box<dyn Error + Send + Sync> }` | A progress-bar template string was not valid; wraps the underlying `indicatif` template error (`lib.rs`). `#[non_exhaustive]`. | `progress-bar` | yes (boxed source) |
| `Io(std::io::Error)` | Wraps a `std::io::Error`. Constructed directly and via `From<std::io::Error>`. | none | no (concrete `std::io::Error`) |
| `Json(Box<dyn Error + Send + Sync>)` | `serde_json` failure, only via `From<serde_json::Error>`. | none | yes (boxed) |
| `Transport(Box<dyn Error + Send + Sync>)` | The request could not be completed (connection/TLS/timeout/transport failure). Only via `From<reqwest::Error>` (`reqwest` feature) or `From<ureq::Error>` (`ureq` feature). A bare `?` on a client call lands here only when the error is not a status-code error. | none for the variant; the `From` impls are gated on `reqwest` / `ureq` | yes (boxed) |
| `SemVer(Box<dyn Error + Send + Sync>)` | `semver` parse failure, only via `From<semver::Error>`. | none | yes (boxed) |
| `Zip(Box<dyn Error + Send + Sync>)` | `zip` archive error, only via `From<ZipError>`. | `archive-zip` | yes (boxed) |
| `ArchiveNotEnabled(String)` | Archive extension whose `archive-*` feature is not enabled. String is the extension (`"zip"`/`"tar"`). | none | no (String) |
| `CompressionNotEnabled(String)` | The asset is compressed with a codec whose feature is not enabled (`lib.rs`). String is the codec token (`"gz"`); enable `compression-tar-gz` to decode it. Distinct from `ArchiveNotEnabled`, which concerns the container format; without this a gzip asset would install its still-compressed bytes as the binary. | none | no (String) |
| `NoSignatures(crate::ArchiveKind)` | Archive contains no signatures to verify. | `signatures` | no (carries `ArchiveKind`) |
| `Signature(Box<dyn Error + Send + Sync>)` | Signature-verification failure, only via `From<ZipsignError>`. | `signatures` | yes (boxed) |
| `InvalidAssetName { name: String }` | The server-supplied asset name is empty, `.`, `..`, contains a `/` or `\` path separator, contains a control character, or is an absolute path; the file is never created (`update.rs`). `#[non_exhaustive]`. | none | no (struct fields) |
| `SignatureNonUTF8` | Generated archive path contains non-UTF-8 characters so its signature cannot be verified. Unit variant. | `signatures` | no (unit) |
| `S3Auth(Box<dyn Error + Send + Sync>)` | S3 SigV4 request-signing failure, including the host-extraction case (a signed URL with no extractable host). Via `From<SystemTimeError>`, `From<hmac::digest::InvalidLength>`, `From<url::ParseError>`, `From<time::error::ComponentRange>`, and direct construction at the host-extraction sites (`s3.rs`). | `s3-auth` | yes (boxed) |
| `InvalidAssetKeyPattern { source: Box<dyn Error + Send + Sync> }` | A user-supplied `asset_key_pattern` on the s3 builders did not compile or lacks a required named capture group (`name` / `version`). Raised from `build()` via `compile_asset_key_pattern` (`s3.rs`); the source is the regex-compile error or a `MessageError` naming the missing group. `#[non_exhaustive]`. | `s3` | yes (boxed source) |

### Reclassification of construction sites

The 1.0 status work split the HTTP-status variants. The three remaining stringly-typed catch-alls
(`Update(String)`, `Release(String)`, `Config(String)`) were then structured, and the
construction sites that stringified-and-discarded a real underlying error now carry a boxed
`source`.

`Update(String)` was split:

- **`update.rs` `install_binary()`** (verify callback returned `Err(..)`) -> `VerificationRejected
  { reason }`. A user-controlled rejection, not an internal failure.
- **`lib.rs` extractor / extract helpers** (no file-name, path not in archive, non-UTF-8 path) ->
  `Internal { message, source: None }`. Internal invariants.
- **`backends/custom.rs` `Blocking`** and **`update.rs` finish-update** (tokio join failure) ->
  `Internal { message, source: Some(JoinError) }`. The `JoinError` is now carried as `source`
  (was previously stringified and discarded).

`Release(String)` was split:

- **`update.rs` `resolve_and_confirm()`** (no asset for target) -> `NoReleaseFound { target:
  Some(...) }`.
- **`github.rs` / `gitlab.rs` / `gitea.rs` / `s3.rs`** (no release / no matching tag / empty
  listing) -> `NoReleaseFound { target: None }`.
- **`github.rs` / `gitlab.rs` / `gitea.rs` `from_value`** (missing payload field) ->
  `MissingAssetField { field }`.
- **`github.rs` / `gitlab.rs` / `gitea.rs`** (malformed non-array listing body) ->
  `InvalidResponse { source }`. Previously mapped to `NoReleaseFound`; a body the crate cannot
  parse is a parse failure, not a clean empty result.
- **`s3.rs`** (listing regex build failure, XML parse failure) -> `InvalidResponse { source }`.
  The underlying error is now carried as `source` (was previously stringified and discarded).

`Config(String)` was split:

- **`common.rs` / `update.rs` / `custom.rs` / `github.rs` / `gitlab.rs` / `gitea.rs` / `s3.rs`**
  (required field unset) -> `MissingField { field }`.
- **`common.rs` `check()` and `lib.rs` `Download` (deferred from `request_header`, surfaced by
  `download_to`)** (invalid request header) -> `InvalidHeader { source }`.
- **`github.rs` / `gitlab.rs` / `gitea.rs` / `update.rs` `api_headers`** (auth token not a valid
  header value) -> `InvalidAuthToken { source }`. The header-parse error is now carried as
  `source` (was previously stringified and discarded).
- **`s3.rs` SigV4 host extraction** (`s3-auth`) -> `S3Auth` (a signing-path failure, grouped
  with the other SigV4 errors).
- **`common.rs` `RequestConfig::check()`** (root-certificate/client-build failure) ->
  `InvalidCertificate { source }`.
- **`lib.rs` `Download::download_to` and `Download::download_to_async`** (same cert/build
  failure when custom root CAs are supplied) -> `InvalidCertificate { source }`.
- **`lib.rs` progress-bar template parse** (`progress-bar`) -> `InvalidProgressStyle { source }`.

`Config(String)` is fully removed; every former producer routes to a structured variant.

Other (unchanged) reclassifications from the status work: a checksum mismatch is
`ChecksumMismatch { expected, computed }` (`checksum.rs`), and a declined confirmation prompt is
`Aborted` (`lib.rs` `confirm()`).

### Display strings

Display strings are human-facing and **may change between minor releases**. For programmatic
decisions, match on variants or use `http_status()` / `url()` rather than parsing the Display
output.

Each variant renders with a specific Display string:

- `Internal { message, .. }` -> `"InternalError: {message}"`
- `VerificationRejected { reason: None }` -> `"VerificationRejectedError: post-update verification rejected the new binary"`; with `Some(r)` it appends `": {r}"`
- `ChecksumMismatch { expected, computed }` -> `"ChecksumMismatchError: checksum mismatch (expected {expected}, computed {computed})"`
- `Aborted` -> `"AbortedError: the update was not confirmed"`
- `NotFound { url }` -> `"NotFoundError: no resource found at {url} (HTTP 404)"`
- `Unauthorized { status, url }` -> `"UnauthorizedError: request to {url} was not authorized (HTTP {status})"`
- `RateLimited { status, url, reset_at, retry_after }` -> `"RateLimitedError: request to {url} was rate limited (HTTP {status})"`, then a wait clause when one is known: `"; retry in {n}s"` when `retry_after` is `Some` (a requested back-off, not necessarily proof the quota is spent -- GitHub's secondary rate limit can answer 403 + `Retry-After` while the quota is still nonzero), else `"; quota resets in {n}s"` when `reset_at` resolves to a still-future wait (omitted when neither field yields a wait, or the window has already elapsed), then always `"; set an auth token to raise the limit, or check less often"`. The two clauses are joined with `"; "` throughout, not a comma before the wait and a colon before the remedy.
- `HttpStatus { status, url }` -> `"HttpStatusError: request to {url} failed with status {status}"`
- `NoReleaseFound { target: None }` -> `"ReleaseError: no release was found"`; with `Some(t)` -> `"ReleaseError: no release found with an asset for target \`{t}\`"`
- `MissingAssetField { field }` -> `"ReleaseError: release/asset payload missing \`{field}\`"`
- `InvalidResponse { source }` -> `"ReleaseError: invalid response: {source}"`
- `MissingField { field }` -> `"ConfigError: \`{field}\` required"`
- `InstallPathNotWritable { path }` -> `"InstallPathNotWritableError: cannot write to install path {path}: run with elevated privileges or choose a user-writable bin_install_path"`
- `NoAppBundle { exe }` -> ``"ConfigError: no `.app` ancestor of {exe}; set bundle_install_path explicitly"``
- `ConflictingConfig { field, conflict }` -> ``"ConfigError: `{field}` conflicts with `{conflict}`; set one or the other"``
- `AppTranslocated { exe }` -> `"AppTranslocatedError: {exe} is running from a translocated (quarantined) copy on a read-only mount, so its bundle cannot be replaced: move the app (e.g. to /Applications) and relaunch it before updating"`
- `InvalidHeader { source }` -> `"ConfigError: invalid HTTP header: {source}"`
- `InvalidAuthToken { source }` -> `"ConfigError: failed to parse auth token: {source}"`
- `InvalidCertificate { source }` -> `"ConfigError: invalid root certificate: {source}"`
- `InvalidProgressStyle { source }` -> `"ConfigError: invalid progress bar template: {source}"` (`progress-bar`)
- `Io(e)` -> `"IoError: {e}"`
- `Json(e)` -> `"JsonError: {e}"` (dereferences the box)
- `Transport(e)` -> `"TransportError: {e}"` (dereferences the box)
- `SemVer(e)` -> `"SemVerError: {e}"` (dereferences the box)
- `Zip(e)` -> `"ZipError: {e}"` (dereferences the box, `archive-zip`)
- `ArchiveNotEnabled(s)` -> `"ArchiveNotEnabledError: Archive extension '{s}' not supported, please enable 'archive-{s}' feature!"`
- `CompressionNotEnabled(s)` -> `"CompressionNotEnabledError: '{s}' compression not supported, please enable the 'compression-tar-gz' feature (a \`.tar.gz\` also needs 'archive-tar')"`
- `InvalidAssetName { name }` -> `"InvalidAssetNameError: unsafe asset name: {name:?}"` (Debug-quoted name)
- `NoSignatures(kind)` -> `"SignatureError: signature verification is only implemented for \`.tar.gz\` and \`.zip\` assets, not {kind} files"` (`signatures`)
- `Signature(e)` -> `"SignatureError: {e}"` (dereferences the box, `signatures`)
- `SignatureNonUTF8` -> `"SignatureError: cannot verify signature of a file with a non-UTF-8 name"` (`signatures`)
- `S3Auth(e)` -> `"S3AuthError: {e}"` (dereferences the box, `s3-auth`)
- `InvalidAssetKeyPattern { source }` -> `"ConfigError: invalid asset_key_pattern: {source}"` (`s3`)

Note: `ArchiveNotEnabled` was corrected from `"ArchiveNotEnabled: ..."` to `"ArchiveNotEnabledError: ..."`;
`SignatureNonUTF8` was corrected from the bare message to `"SignatureError: ..."`, consistent with
every other variant using a `<Name>Error:` prefix.

### source() and downcast

`source()` returns the inner error for the wrapping variants: `Io` (the concrete io error); the
boxed `Json`, `Transport`, `SemVer`, `Zip` (gated), `Signature` (gated), `S3Auth` (gated); the
boxed-source variants `InvalidResponse`, `InvalidHeader`, `InvalidAuthToken`,
`InvalidCertificate`, `InvalidProgressStyle` (gated), `InvalidAssetKeyPattern` (gated); and
`Internal` when its `source` is `Some`
-- each via deref of the box. The `Internal { source: None }` form and all field-only variants
(`VerificationRejected`, `ChecksumMismatch`, `Aborted`, `NotFound`, `Unauthorized`, `HttpStatus`,
`RateLimited`, `NoReleaseFound`, `MissingAssetField`, `MissingField`, `InstallPathNotWritable`, `NoAppBundle`,
`ConflictingConfig`, `AppTranslocated`, `ArchiveNotEnabled`, `CompressionNotEnabled`,
`InvalidAssetName`, `NoSignatures`, `SignatureNonUTF8`) return `None`. The concrete inner error of
a boxed variant is reachable at runtime through `source()` and `downcast_ref::<ConcreteType>()`
(e.g. `err.source().and_then(|s| s.downcast_ref::<reqwest::Error>())`).

`InvalidHeader`'s `source` is a crate-internal `MessageError` (a small owned message error), not a
dependency type, because the builder header path discards the unnameable generic `TryInto`
conversion error. The `InvalidAuthToken` and `InvalidResponse` sources are the real underlying
errors (a header-value parse error, a quick-xml reader error, or a regex build error).

### http_status() helper

```rust
pub fn http_status(&self) -> Option<u16>
```

(`errors.rs:373-381`.) Returns the HTTP status code when the error came from a completed non-2xx
response:
- `NotFound { .. }` -> `Some(404)`
- `Unauthorized { status, .. }` -> `Some(status)`
- `RateLimited { status, .. }` -> `Some(status)`
- `HttpStatus { status, .. }` -> `Some(status)`
- all other variants -> `None`

### url() accessor

```rust
pub fn url(&self) -> Option<&str>
```

(`errors.rs:385-393`.) Returns the request URL for the HTTP error variants; `None` for everything
else:
- `NotFound { url }` -> `Some(url)`
- `Unauthorized { url, .. }` -> `Some(url)`
- `RateLimited { url, .. }` -> `Some(url)`
- `HttpStatus { url, .. }` -> `Some(url)`
- all other variants -> `None`

### rate_limit_delay() helper

```rust
pub fn rate_limit_delay(&self) -> Option<std::time::Duration>
```

(`errors.rs:417-428`.) `None` for every variant except `RateLimited`. Returns how long to wait,
measured from now, before retrying: `retry_after` when present, else `reset_at` minus the current
time (`None` when that difference would be negative, i.e. the window has already elapsed). This is
the single place the `Retry-After`-then-`reset_at` precedence is computed -- neither field alone is
correct on its own: GitHub's *primary* rate limit sends only `x-ratelimit-reset`, so treating a
missing `retry_after` as a zero wait would spend more quota immediately, while naively subtracting
an elapsed `reset_at` from now would underflow/panic. `Display`'s optional wait clause (see the
`RateLimited` row above) calls this same accessor, so the rendered string and a caller's
programmatic back-off can never disagree. Both source values are capped at 24h before they
ever reach this accessor (`MAX_RATE_LIMIT_WAIT`, `errors.rs:882`; see `parse_reset_epoch`,
`errors.rs:926-933`, and `parse_retry_after`, `errors.rs:946-949`), so a hostile or malformed
response cannot use this accessor to park a caller indefinitely. `parse_retry_after` also floors a
zero-second `Retry-After` to `None` (a separate rule from the 24h ceiling): see "HTTP status
construction mapping" below.

### HTTP status construction mapping (both clients)

Both `reqwest` and `ureq` clients call `errors::status_to_error_with_headers(status_code, url, headers)`
(`errors.rs:966-981`), which reads the rate-limit signals off `headers` into a `RateLimitSignals`
and delegates to the pure `classify_status(status_code, url, signals)` (`errors.rs:899-921`), which
classifies the rate-limit case first and otherwise delegates to `status_to_error(status_code, url)`
(`errors.rs:844-857`):
- 429 -> `Error::RateLimited { status, url, reset_at, retry_after }`, **always**, with or without
  any quota headers.
- 403 whose remaining-quota header parses as `0`, **or** whose `Retry-After` header parses to a
  nonzero delay (within the 24h ceiling) -> `Error::RateLimited { .. }`. The `Retry-After` branch
  covers GitHub's *secondary* rate limit, which can answer 403 + `Retry-After` while the
  remaining-quota header is still nonzero. A `Retry-After: 0` does **not** satisfy this branch
  (`parse_retry_after` treats a zero delay as no signal, `errors.rs:946-949`): a bare 403 carrying
  only a zero `Retry-After` stays `Unauthorized` rather than becoming a `RateLimited` with a
  zero-second wait, which would otherwise mask a genuine authorization failure and make a caller
  following this crate's own sleep-then-continue guidance spin in a tight loop.
- 404 -> `Error::NotFound { url }`
- 401 or a 403 with neither of the above signals -> `Error::Unauthorized { status, url }`
- any other non-2xx -> `Error::HttpStatus { status, url }`

The remaining-quota and reset signals are read from `x-ratelimit-remaining` / `x-ratelimit-reset`
falling back to `ratelimit-remaining` / `ratelimit-reset` (`errors.rs:976-977`), and the delay from
`Retry-After`. **Why the fallback is needed at all:** `HeaderMap` lookups are already
case-insensitive, so a single lookup key matches every casing of a *given* header name (e.g. it is
why `RateLimit-Remaining` matches a lookup for `ratelimit-remaining`); that alone does not bridge
github/gitea/gitee's `x-ratelimit-*` name and gitlab's *differently spelled* `RateLimit-*` name --
those are two distinct header names, and it is the explicit `.or_else(...)` chain, not case
insensitivity, that reads both. A 403 with none of the rate-limit signals keeps its `Unauthorized`
classification; a 429 is never `Unauthorized` or `HttpStatus`, only `RateLimited`. Both `reset_at`
and `retry_after` are capped at 24h (`MAX_RATE_LIMIT_WAIT`, `errors.rs:882`): a value beyond the
ceiling resolves to `None` rather than being clamped down to it.

For ureq specifically (`http_client/ureq.rs`), all three lanes now classify a given status +
headers identically:
- The **default (built-in) per-call agent** is built with `.http_status_as_error(false)`
  (`build_call_agent`, `ureq.rs:75`) so ureq does not short-circuit on non-2xx, and the explicit
  `!res.status().is_success()` check at the bottom of `get` runs with `res.status().as_u16()` and
  `res.headers()` feeding `status_to_error_with_headers` (`ureq.rs:202-208`).
- An **injected agent** (caller-supplied) keeps ureq's own default `http_status_as_error(true)` at
  the agent level, but `get` applies a **per-request** override on the request builder when the
  agent's own config has not already disabled ureq's status-error (`needs_status_override`,
  `ureq.rs:122-124`) -- `req.config().http_status_as_error(false).build()` (`ureq.rs:175`, inside
  the conditional block at `ureq.rs:156-176`) -- before calling it. This does not touch the injected
  agent's own persistent timeout/TLS/proxy configuration, only this request's status handling, and
  it means an injected agent's non-2xx response reaches the same
  header-aware `status_to_error_with_headers` check as the default agent (`ureq.rs:202-208`), so it
  **can** and does reach `RateLimited`. The `Err(ureq::Error::StatusCode(code)) if is_injected` arm
  (`ureq.rs:184-198`), which maps via the header-less `status_to_error(code, url)` (a 429 there is
  still `RateLimited`, carrying no wait; a 403 there stays `Unauthorized`, since only a header can
  tell a spent quota from a credential failure), is retained only as a **defensive fallback** for a
  future ureq that
  might stop honoring the per-request override; it is not expected to fire in normal operation. All
  other `ureq::Error` variants are transport failures and map to `Error::Transport` via `From`.

### Why boxed

`Transport`, `S3Auth`, `Zip`, `Signature`, `Json`, `SemVer`, and the structured-source variants
`InvalidResponse` / `InvalidHeader` / `InvalidAuthToken` / `InvalidCertificate` /
`InvalidProgressStyle` (and `Internal`'s optional `source`) wrap
`Box<dyn std::error::Error + Send + Sync>` so no dependency type appears in the public API. The
inner type can change (reqwest vs ureq selection, a `zip`/`serde_json`/`semver` major bump, the
signing implementation, the XML/regex/header dependency) without altering the public surface.
Inspection is still possible via `source()` + downcast. (`Io` is the exception: it carries the std
type directly, since `std::io::Error` is stable std.)

## Public surface

- `pub enum Error` with the variants above; `#[non_exhaustive]`.
- `pub type Result<T> = std::result::Result<T, Error>;` (`errors.rs:8`).
- `pub fn http_status(&self) -> Option<u16>` inherent method on `Error`.
- `pub fn url(&self) -> Option<&str>` inherent method on `Error`.
- `pub fn rate_limit_delay(&self) -> Option<std::time::Duration>` inherent method on `Error`
  (`errors.rs:417-428`); `None` except for `RateLimited`.
- Public constructors for custom `ReleaseSource` implementors (the release-flow variants are
  `#[non_exhaustive]`, so downstream code cannot build them with a struct literal):
  `Error::no_release_found()` and `Error::no_release_found_for_target(target: impl Into<String>)`,
  `Error::missing_asset_field(field: impl Into<String>)`,
  `Error::invalid_response(source: impl Into<Box<dyn Error + Send + Sync>>)`,
  `Error::http_status_error(status: u16, url: impl Into<String>)` (routes through
  `status_to_error`, so 404 -> `NotFound`, 401/403 -> `Unauthorized`, and 429 -> `RateLimited` with
  both wait fields `None`; having no headers to read, it cannot promote a *403* to `RateLimited`),
  `Error::http_status_error_with_headers(status: u16, url: impl Into<String>, headers: &HeaderMap)`
  (the header-aware form, for a custom `HttpClient` that has the response in hand), and
  `Error::checksum_mismatch(expected: impl Into<String>, computed: impl Into<String>)`.
- Trait impls: `Debug` (derived), `Display`, `std::error::Error` (with `source()`).
- `From` impls: `std::io::Error`, `serde_json::Error`, `semver::Error` (always); `reqwest::Error`
  (`reqwest`), `ureq::Error` (`ureq`), `ZipError` (`archive-zip`), `ZipsignError` (`signatures`);
  and for `s3-auth`: `SystemTimeError`, `hmac::digest::InvalidLength`, `url::ParseError`,
  `time::error::ComponentRange`.
- `pub(crate) fn status_to_error(status: u16, url: &str) -> Error` (`errors.rs`) maps a status
  code to `NotFound` / `Unauthorized` / `RateLimited` (429 only, no wait fields) / `HttpStatus`.
  429 does not need a header to be rate limiting (RFC 6585), so the header-blind and header-aware
  paths agree on it; the header-aware path only adds the wait fields.
- `pub(crate) fn status_to_error_with_headers(status: u16, url: &str, headers: &http::HeaderMap) -> Error`
  (`errors.rs`) reads the rate-limit headers off a response and delegates to the pure
  `classify_status(status, url, RateLimitSignals)`, which is what the built-in clients call.
- `pub(crate) struct MessageError(String)` (`errors.rs`): a minimal owned message error used as the
  boxed `source` of `InvalidHeader` where the underlying `TryInto` conversion error is not
  nameable. Crate-internal, not part of the public surface.

## Invariants and regression checklist

- `Error` is `#[non_exhaustive]`: downstream `match` must include a wildcard arm; new variants are
  not a breaking change.
- The opaque variants (`Json`, `Transport`, `SemVer`, `Zip`, `Signature`, `S3Auth`) expose their
  inner error via `source()` (deref of the box), and `Display` embeds the inner message with the
  `<Name>Error:` prefix.
- No public dependency type leaks: the wrapping variants are `Box<dyn Error + Send + Sync>`, never a
  concrete `reqwest` / `ureq` / `zip` / `serde_json` / `semver` / `zipsign` type. `Io` deliberately
  carries the std `io::Error`.
- `Transport` = the request could not be completed (connection/TLS/timeout); `NotFound` /
  `Unauthorized` / `RateLimited` / `HttpStatus` = the request completed but returned a non-2xx
  status.
- Both reqwest and ureq produce identical structured status variants for any given HTTP status code.
  The old reqwest=`Network` / ureq=`Http` inconsistency (documented in the now-superseded
  `error-network-vs-http-semantics.md`) is resolved.
- 404 -> `NotFound`; 401 or a 403 with neither rate-limit signal -> `Unauthorized`; any other
  non-2xx -> `HttpStatus`; except that 429 is **always** `RateLimited`, and a 403 carrying a zero
  remaining-quota header **or** a usable `Retry-After` is also `RateLimited` -- the rate-limit
  check runs first.
- A 403 with neither a zero remaining-quota header nor a usable `Retry-After` must stay
  `Unauthorized` (a genuine credential failure); this is the only carve-out left after the
  broadened rule -- a 429 is never `Unauthorized` or `HttpStatus`, only `RateLimited`, with or
  without any quota headers at all.
- `reset_at` and `retry_after` are each capped at 24h (`MAX_RATE_LIMIT_WAIT`); a server-supplied
  value beyond the ceiling resolves to `None`, never a clamped-down duration.
- A `Retry-After: 0` is floored to `None` by `parse_retry_after`, the same as an absent or
  unparseable header: `classify_status`'s 403 branch keys on `retry_after.is_some()`, so a literal
  zero delay would otherwise promote a bare authorization failure to a `RateLimited` carrying a
  zero-second wait. A 429 is unaffected by this floor -- it classifies as `RateLimited` on the
  status code alone, whatever `Retry-After` says.
- `Error::rate_limit_delay()` is the one place the `Retry-After`-then-`reset_at` precedence is
  computed; `Display`'s optional wait clause calls it rather than re-deriving the choice.
- The ureq injected-agent path is **not** an exception to the identical-classification rule: a
  per-request `http_status_as_error(false)` override makes it reach the header-aware check the
  same as every other lane, so it can and does produce `RateLimited`.
- `http_status()` returns `Some(u16)` for `NotFound`/`Unauthorized`/`RateLimited`/`HttpStatus`;
  `None` for all other variants.
- `url()` returns `Some(&str)` for `NotFound`/`Unauthorized`/`RateLimited`/`HttpStatus`; `None` for
  all other variants. The `RateLimited` url is redacted like the others.
- A checksum digest mismatch produces `Error::ChecksumMismatch { expected, computed }`. Both
  fields are lowercase hex-encoded digests.
- A user-declined confirmation prompt produces `Error::Aborted`.
- Every struct-form variant carries `#[non_exhaustive]` on the variant (`Unauthorized`,
  `RateLimited`, `HttpStatus`, `Internal`, `VerificationRejected`, `NoReleaseFound`, `MissingAssetField`,
  `InvalidResponse`, `MissingField`, `InstallPathNotWritable`, `InvalidHeader`, `InvalidAuthToken`,
  `InvalidCertificate`, `InvalidProgressStyle`, `InvalidAssetName`, `NotFound`,
  `ChecksumMismatch`, `NoAppBundle`, `ConflictingConfig`, `AppTranslocated`).
- The bundle-mode config variants are raised from `build()`, before any request: `NoAppBundle`
  (macOS, no `.app` ancestor to derive `bundle_install_path` from), `ConflictingConfig` (bundle mode
  plus an explicit `bin_install_path`/`bin_path_in_archive`), and `AppTranslocated` (a quarantined
  app running from a read-only translocated mount). Off macOS, bundle mode without an explicit
  `bundle_install_path` is `MissingField { field: "bundle_install_path" }` instead.
- `Error::Internal` is reserved for genuine internal/invariant failures: extractor invariants,
  archive-path failures, and tokio blocking-task join failures (which carry the `JoinError` as
  `source`).
- A rejecting `verify_binary` callback produces `Error::VerificationRejected { reason: Some(<error message>) }`.
- The sites that previously stringified-and-discarded a source now chain it via `source()`: the
  S3 XML/regex parse (`InvalidResponse`), the auth-token header-value parse (`InvalidAuthToken`),
  and the tokio `JoinError` sites (`Internal`).
- `Error::InstallPathNotWritable { path }` names the `bin_install_path` that could not be
  written. It carries no source and exposes no `http_status()`/`url()`. Display prefix is
  `"InstallPathNotWritableError: "`. Raised by the opt-in preflight probe
  (`check_install_path_writable(true)`) on a definite `PermissionDenied`, and always by the
  install step on a permission failure. Other install-step IO errors map to `Error::Io` with the
  path embedded in the message, `ErrorKind` preserved.
- `Error::Config(String)` no longer exists. Its former producers route to structured variants:
  the `s3-auth` SigV4 host-extraction site (`s3.rs`) -> `S3Auth`; the root-certificate/client-build
  failures in `RequestConfig::check()` (`common.rs`) and `Download::download_to` /
  `download_to_async` (`lib.rs`) -> `InvalidCertificate { source }`.
- A malformed (non-array) release-listing body maps to `InvalidResponse`, not `NoReleaseFound`.
- A gzip asset with `compression-tar-gz` off produces `Error::CompressionNotEnabled("gz")`
  instead of installing the still-compressed bytes.
- An unsafe server-supplied asset name (empty, `.`/`..`, path separators, control characters,
  absolute path) produces `Error::InvalidAssetName { name }` before any file is created.
- `ChecksumMismatch` is compiled unconditionally (no feature gate).
- Custom sources build the release-flow variants through the public constructors
  (`no_release_found` / `no_release_found_for_target`, `missing_asset_field`,
  `invalid_response`, `http_status_error`, `checksum_mismatch`), not struct literals.
- The signatures-gated unit variant is named `SignatureNonUTF8`; its Display is
  `"SignatureError: cannot verify signature of a file with a non-UTF-8 name"`.
- `ArchiveNotEnabled` Display starts with `"ArchiveNotEnabledError: "`.

## Tests

`errors.rs` (`mod tests`): each boxed variant is asserted opaque-with-`source()` and its
`Display` prefix + embedded inner message (`Json`, `SemVer`, `Zip` gated, `Signature` gated);
`reqwest_error_maps_to_transport_variant` and `ureq_error_maps_to_transport_variant` pin the
`From<*::Error>` -> `Transport` mapping per client; `not_found_display_matches_spec`,
`unauthorized_display_matches_spec_401`, `unauthorized_display_matches_spec_403`,
`http_status_display_matches_spec` pin the exact Display strings; `http_status_helper_*` tests pin
`http_status()` return values; `status_to_error_*` tests pin the 404/401/403/500/503 mapping;
`signature_non_utf8_variant_is_renamed_and_displays` pins the rename and updated message;
`checksum_mismatch_display_exact_string`, `checksum_mismatch_http_status_is_none`,
`checksum_mismatch_url_is_none` pin the new `ChecksumMismatch` variant; `aborted_display_exact_string`,
`aborted_http_status_is_none`, `aborted_url_is_none` pin `Aborted`; `url_helper_*` tests pin the
`url()` accessor; `archive_not_enabled_display_has_correct_prefix` and
`signature_non_utf8_display_has_signature_error_prefix` pin the corrected prefixes.

Rate-limit classification (`errors.rs` `mod tests`): `classify_status_maps_a_spent_quota_403_to_rate_limited`,
`classify_status_keeps_a_plain_403_unauthorized`, `classify_status_keeps_403_unauthorized_when_quota_remains`,
`classify_status_maps_a_spent_quota_429_to_rate_limited`, `classify_status_maps_a_bare_429_to_rate_limited`,
`classify_status_maps_a_429_with_only_retry_after_to_rate_limited`,
`classify_status_maps_a_403_with_retry_after_to_rate_limited`,
`classify_status_maps_a_403_with_spent_quota_and_no_retry_after_to_rate_limited`,
`classify_status_ignores_quota_headers_on_a_404`, `classify_status_ignores_quota_headers_on_other_statuses`,
`classify_status_tolerates_unparseable_reset_and_retry_after`, `classify_status_redacts_the_rate_limited_url`,
`status_to_error_with_headers_reads_both_header_spellings`, `http_status_and_url_helpers_cover_rate_limited`,
`rate_limited_display_names_the_limit_and_the_remedy`,
`rate_limited_display_omits_the_wait_when_unknown_or_elapsed`,
`rate_limited_display_separates_its_clauses_consistently`. The 24h clamp:
`parse_retry_after_keeps_a_normal_delay`, `parse_retry_after_clamps_at_twenty_four_hours`,
`parse_retry_after_rejects_the_u64_max_delay`, `classify_status_ignores_an_over_ceiling_retry_after_on_a_403`.
The zero-`Retry-After` floor: `parse_retry_after_treats_a_zero_delay_as_no_signal`,
`classify_status_keeps_a_403_with_zero_retry_after_unauthorized`,
`classify_status_still_rate_limits_a_spent_quota_403_with_zero_retry_after` (a zero delay does not
undo a spent-quota classification), `classify_status_keeps_a_429_with_zero_retry_after_rate_limited`
(the floor does not touch the always-`RateLimited` 429 rule).
`rate_limit_delay()`: `rate_limit_delay_prefers_retry_after_over_reset_at`,
`rate_limit_delay_uses_retry_after_alone`, `rate_limit_delay_derives_a_wait_from_a_future_reset_at`,
`rate_limit_delay_is_none_for_an_elapsed_reset_at`, `rate_limit_delay_is_none_when_nothing_is_known`,
`rate_limit_delay_is_none_for_a_non_rate_limited_variant`.

The ureq injected-agent classification gap closure is pinned in `http_client/ureq.rs` (`mod tests`):
`injected_agent_sees_rate_limit_headers`, `injected_agent_with_status_error_disabled_sees_rate_limit_headers`,
`injected_agent_no_status_error_falls_through_to_is_success_check`,
`default_agent_path_maps_statuses_identically_to_injected`. The retry short-circuit on `RateLimited`
is pinned in `backends/mod.rs` (`mod tests`): `retry_does_not_retry_a_rate_limited_error`,
`retry_still_consumes_the_budget_for_a_non_rate_limited_error`,
`retry_async_does_not_retry_a_rate_limited_error`,
`retry_async_still_consumes_the_budget_for_a_non_rate_limited_error`.

`checksum.rs` (`mod tests`): `mismatch_yields_checksum_mismatch_variant` asserts that a digest
mismatch through `Checksum::verify()` produces `Error::ChecksumMismatch` with the correct
`expected` and `computed` fields; `mismatch_display_contains_expected_and_computed` pins the
Display string.

Variant-routing is asserted across the backends: `InvalidHeader`/`MissingField` from invalid
headers / missing fields (`common.rs`, `github.rs`, `gitlab.rs`, `gitea.rs`, `s3.rs`),
`NoReleaseFound`/`MissingAssetField` from missing/empty payloads, `InvalidResponse` from
non-array listing bodies (`github.rs`, `gitlab.rs`, `gitea.rs`) and malformed XML (`s3.rs`),
`InvalidCertificate` from a bad root certificate (`common.rs`, `github.rs`),
`NotFound`/`Unauthorized`/`HttpStatus` on non-2xx (both clients produce the same variant,
asserted in `github.rs`, `gitlab.rs`, `s3.rs`), `S3Auth` from the hostless-signed-URL case
(`s3.rs`), and `HttpStatus` propagation through pagination/retry (`backends/mod.rs`).

## Related

- `error-variant-granularity.md`
- `1.0-api-surface.md`
- `ref-update-pipeline.md`

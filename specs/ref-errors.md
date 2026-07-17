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
| `HttpStatus { status: u16, url: String }` | A request completed and returned any other non-2xx status (e.g. 500, 503). Raised by both HTTP clients. `#[non_exhaustive]`. | none | no (struct fields) |
| `NoReleaseFound { target: Option<String> }` | The clean negative of a release lookup: no release / no matching release for a tag/version (`github.rs`, `gitlab.rs`, `gitea.rs`, `s3.rs`), or the resolved release had no asset for the requested target (`update.rs`, with `target: Some(...)`). `#[non_exhaustive]`. | none | no (struct fields) |
| `MissingAssetField { field: String }` | A release/asset payload was missing a required field (`url`/`name`/`tag_name`/`created_at`/`assets`/`browser_download_url`/`assets.links`) in each backend's DTO conversion (`github.rs`, `gitlab.rs`, `gitea.rs`). `String` so a custom source can report a dynamic field path (e.g. `assets[2].url`). `#[non_exhaustive]`. | none | no (struct fields) |
| `InvalidResponse { source: Box<dyn Error + Send + Sync> }` | A backend response could not be parsed: a malformed (non-array) JSON release-listing body (`github.rs`, `gitlab.rs`, `gitea.rs`), the S3 listing regex build failure, and the S3 XML parse failure (`s3.rs`). The underlying error is carried as `source`. `#[non_exhaustive]`. | none | yes (boxed source) |
| `MissingField { field: &'static str }` | A required builder/configuration field was not set: `current_version`/`bin_name`/`bin_path_in_archive` (`common.rs`), `version` (`update.rs`), `source` (`custom.rs`), `repo_owner`/`repo_name` (`github.rs`, `gitlab.rs`, `gitea.rs`), `host` (`gitea.rs`), `bucket_name`/`region` (`s3.rs`). `#[non_exhaustive]`. | none | no (struct fields) |
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
| `InvalidAssetName { name: String }` | The server-supplied asset name is empty, `.`, `..`, contains a `/` or `\` path separator, or is an absolute path; the file is never created (`update.rs`). `#[non_exhaustive]`. | none | no (struct fields) |
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
- `HttpStatus { status, url }` -> `"HttpStatusError: request to {url} failed with status {status}"`
- `NoReleaseFound { target: None }` -> `"ReleaseError: no release was found"`; with `Some(t)` -> `"ReleaseError: no release found with an asset for target \`{t}\`"`
- `MissingAssetField { field }` -> `"ReleaseError: release/asset payload missing \`{field}\`"`
- `InvalidResponse { source }` -> `"ReleaseError: invalid response: {source}"`
- `MissingField { field }` -> `"ConfigError: \`{field}\` required"`
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
`NoReleaseFound`, `MissingAssetField`, `MissingField`, `ArchiveNotEnabled`,
`CompressionNotEnabled`, `InvalidAssetName`, `NoSignatures`, `SignatureNonUTF8`) return `None`. The concrete inner error of
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

Returns the HTTP status code when the error came from a completed non-2xx response:
- `NotFound { .. }` -> `Some(404)`
- `Unauthorized { status, .. }` -> `Some(status)`
- `HttpStatus { status, .. }` -> `Some(status)`
- all other variants -> `None`

### url() accessor

```rust
pub fn url(&self) -> Option<&str>
```

Returns the request URL for the HTTP error variants; `None` for everything else:
- `NotFound { url }` -> `Some(url)`
- `Unauthorized { url, .. }` -> `Some(url)`
- `HttpStatus { url, .. }` -> `Some(url)`
- all other variants -> `None`

### HTTP status construction mapping (both clients)

Both `reqwest` and `ureq` clients call `errors::status_to_error(status_code, url)` which maps:
- 404 -> `Error::NotFound { url }`
- 401 or 403 -> `Error::Unauthorized { status, url }`
- any other non-2xx -> `Error::HttpStatus { status, url }`

For ureq specifically (`http_client/ureq.rs`):
- The **default (built-in) agent** is configured with `.http_status_as_error(false)` so ureq does
  not short-circuit on non-2xx, and the explicit `!res.status().is_success()` check runs with
  `res.status().as_u16()` feeding `status_to_error`.
- An **injected agent** (caller-supplied, cannot be reconfigured) may fire `ureq::Error::StatusCode(code)`
  from `call()?`. This arm is caught explicitly and mapped via `status_to_error(code, url)`. All
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
- Public constructors for custom `ReleaseSource` implementors (the release-flow variants are
  `#[non_exhaustive]`, so downstream code cannot build them with a struct literal):
  `Error::no_release_found()` and `Error::no_release_found_for_target(target: impl Into<String>)`,
  `Error::missing_asset_field(field: impl Into<String>)`,
  `Error::invalid_response(source: impl Into<Box<dyn Error + Send + Sync>>)`,
  `Error::http_status_error(status: u16, url: impl Into<String>)` (routes through
  `status_to_error`, so 404 -> `NotFound` and 401/403 -> `Unauthorized`), and
  `Error::checksum_mismatch(expected: impl Into<String>, computed: impl Into<String>)`.
- Trait impls: `Debug` (derived), `Display`, `std::error::Error` (with `source()`).
- `From` impls: `std::io::Error`, `serde_json::Error`, `semver::Error` (always); `reqwest::Error`
  (`reqwest`), `ureq::Error` (`ureq`), `ZipError` (`archive-zip`), `ZipsignError` (`signatures`);
  and for `s3-auth`: `SystemTimeError`, `hmac::digest::InvalidLength`, `url::ParseError`,
  `time::error::ComponentRange`.
- `pub(crate) fn status_to_error(status: u16, url: &str) -> Error` (`errors.rs`) maps a status
  code to `NotFound` / `Unauthorized` / `HttpStatus`.
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
  `Unauthorized` / `HttpStatus` = the request completed but returned a non-2xx status.
- Both reqwest and ureq produce identical structured status variants for any given HTTP status code.
  The old reqwest=`Network` / ureq=`Http` inconsistency (documented in the now-superseded
  `error-network-vs-http-semantics.md`) is resolved.
- 404 -> `NotFound`; 401 or 403 -> `Unauthorized`; any other non-2xx -> `HttpStatus`.
- `http_status()` returns `Some(u16)` for `NotFound`/`Unauthorized`/`HttpStatus`; `None` for
  all other variants.
- `url()` returns `Some(&str)` for `NotFound`/`Unauthorized`/`HttpStatus`; `None` for all other
  variants.
- A checksum digest mismatch produces `Error::ChecksumMismatch { expected, computed }`. Both
  fields are lowercase hex-encoded digests.
- A user-declined confirmation prompt produces `Error::Aborted`.
- Every struct-form variant carries `#[non_exhaustive]` on the variant (`Unauthorized`,
  `HttpStatus`, `Internal`, `VerificationRejected`, `NoReleaseFound`, `MissingAssetField`,
  `InvalidResponse`, `MissingField`, `InvalidHeader`, `InvalidAuthToken`, `InvalidCertificate`,
  `InvalidProgressStyle`, `InvalidAssetName`, `NotFound`, `ChecksumMismatch`).
- `Error::Internal` is reserved for genuine internal/invariant failures: extractor invariants,
  archive-path failures, and tokio blocking-task join failures (which carry the `JoinError` as
  `source`).
- A rejecting `verify_binary` callback produces `Error::VerificationRejected { reason: Some(<error message>) }`.
- The sites that previously stringified-and-discarded a source now chain it via `source()`: the
  S3 XML/regex parse (`InvalidResponse`), the auth-token header-value parse (`InvalidAuthToken`),
  and the tokio `JoinError` sites (`Internal`).
- `Error::Config(String)` no longer exists. Its former producers route to structured variants:
  the `s3-auth` SigV4 host-extraction site (`s3.rs`) -> `S3Auth`; the root-certificate/client-build
  failures in `RequestConfig::check()` (`common.rs`) and `Download::download_to` /
  `download_to_async` (`lib.rs`) -> `InvalidCertificate { source }`.
- A malformed (non-array) release-listing body maps to `InvalidResponse`, not `NoReleaseFound`.
- A gzip asset with `compression-tar-gz` off produces `Error::CompressionNotEnabled("gz")`
  instead of installing the still-compressed bytes.
- An unsafe server-supplied asset name (empty, `.`/`..`, path separators, absolute path) produces
  `Error::InvalidAssetName { name }` before any file is created.
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

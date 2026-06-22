# Error model (reference)

Status: implemented

## Scope

The crate's single public error type `errors::Error` (re-exported as `self_update::errors::Error`),
its `Result<T>` alias, the `Display` / `std::error::Error` (`source()`) impls, the `From`
conversions, the `http_status()` helper, and the `url()` accessor. Source of truth:
`src/errors.rs`. Construction sites are spread across the backends, the HTTP clients, the update
pipeline, and the checksum module.

## Behavior

`Error` is declared `#[derive(Debug)] #[non_exhaustive] pub enum` at `errors.rs`. Every variant,
what produces it, and its feature gate:

| Variant | Produced by | Feature gate | Opaque/boxed? |
| --- | --- | --- | --- |
| `Update(String)` | Genuine internal update failures: extractor source has no file name (`lib.rs`); extract failures (path not in archive, non-UTF-8 archive path); blocking-task join failure (`custom.rs`); post-update verify callback rejected the binary (`update.rs`). | none | no (String) |
| `ChecksumMismatch { expected: String, computed: String }` | The downloaded artifact's digest did not match the configured `Checksum` (`checksum.rs`). Both fields are lowercase hex-encoded digests. | `checksums` | no (struct fields) |
| `Aborted` | The user declined the interactive confirmation prompt (`lib.rs` `confirm()`). | none | no (unit) |
| `NotFound { url: String }` | A request completed and returned HTTP 404. Raised by both HTTP clients when the response status is 404. | none | no (struct fields) |
| `Unauthorized { status: u16, url: String }` | A request completed and returned HTTP 401 or 403. `status` holds the exact code. Raised by both HTTP clients. | none | no (struct fields) |
| `HttpStatus { status: u16, url: String }` | A request completed and returned any other non-2xx status (e.g. 500, 503). Raised by both HTTP clients. | none | no (struct fields) |
| `Release(String)` | Problem with the resolved release: no asset for target (`update.rs`), missing asset/release fields in each backend's `from_value` (`github.rs`, `gitlab.rs`, `gitea.rs`), no releases found, empty/non-array payloads, missing tag (`gitlab.rs`, `gitea.rs`, `s3.rs`), S3 XML/regex parse failure (`s3.rs`). | none | no (String) |
| `Config(String)` | Builder/configuration error: missing required field (`update.rs`, `backends/common.rs`, `custom.rs`), missing `repo_owner`/`repo_name`/`bucket_name`/`region` (`github.rs`, `gitlab.rs`, `gitea.rs`, `s3.rs`), invalid HTTP header name/value (`lib.rs`, surfaced from `build()` via `common.rs`), auth-token parse failure, host-extraction failure (`s3.rs`). | none | no (String) |
| `Io(std::io::Error)` | Wraps a `std::io::Error`. Constructed directly and via `From<std::io::Error>`. | none | no (concrete `std::io::Error`) |
| `Json(Box<dyn Error + Send + Sync>)` | `serde_json` failure, only via `From<serde_json::Error>`. | none | yes (boxed) |
| `Transport(Box<dyn Error + Send + Sync>)` | The request could not be completed (connection/TLS/timeout/transport failure). Only via `From<reqwest::Error>` (`reqwest` feature) or `From<ureq::Error>` (`ureq` feature). A bare `?` on a client call lands here only when the error is not a status-code error. | none for the variant; the `From` impls are gated on `reqwest` / `ureq` | yes (boxed) |
| `SemVer(Box<dyn Error + Send + Sync>)` | `semver` parse failure, only via `From<semver::Error>`. | none | yes (boxed) |
| `Zip(Box<dyn Error + Send + Sync>)` | `zip` archive error, only via `From<ZipError>`. | `archive-zip` | yes (boxed) |
| `ArchiveNotEnabled(String)` | Archive extension whose `archive-*` feature is not enabled. String is the extension (`"zip"`/`"tar"`). | none | no (String) |
| `NoSignatures(crate::ArchiveKind)` | Archive contains no signatures to verify. | `signatures` | no (carries `ArchiveKind`) |
| `Signature(Box<dyn Error + Send + Sync>)` | Signature-verification failure, only via `From<ZipsignError>`. | `signatures` | yes (boxed) |
| `SignatureNonUTF8` | Generated archive path contains non-UTF-8 characters so its signature cannot be verified. Unit variant. | `signatures` | no (unit) |
| `S3Auth(Box<dyn Error + Send + Sync>)` | S3 SigV4 request-signing failure. Via `From<SystemTimeError>`, `From<hmac::digest::InvalidLength>`, `From<url::ParseError>`, `From<time::error::ComponentRange>`. | `s3-auth` | yes (boxed) |

### Reclassification of construction sites

`Error::Update(String)` was previously overloaded to cover three unrelated outcomes. The sites have
been reclassified as follows:

- **`checksum.rs` verify()** (digest mismatch) -- was `Error::Update("... checksum mismatch ...")`
  -- now `Error::ChecksumMismatch { expected, computed }`. The actual expected and computed hex
  digests are carried as fields.
- **`lib.rs` `confirm()`** (user answered "no") -- was `bail!(Error::Update, "Update aborted")`
  -- now `Err(Error::Aborted)`.
- **`backends/custom.rs` `Blocking`** (tokio blocking-task join failure) -- stays `Error::Update`.
  These are internal programming failures (the spawned task panicked or was cancelled), not
  user-visible outcomes.
- **`update.rs` `install_binary()`** (verify callback rejected the binary) -- stays `Error::Update`.
  The callback returning `false` is a genuine internal abort, not a user prompt decline.
- **`lib.rs` extractor / extract helpers** (no file-name, path not found, non-UTF-8 path) -- stays
  `Error::Update`. These are internal / invariant failures in the extraction pipeline.

### Display strings

Display strings are human-facing and **may change between minor releases**. For programmatic
decisions, match on variants or use `http_status()` / `url()` rather than parsing the Display
output.

Each variant renders with a specific Display string:

- `Update(s)` -> `"UpdateError: {s}"`
- `ChecksumMismatch { expected, computed }` -> `"ChecksumMismatchError: checksum mismatch (expected {expected}, computed {computed})"`
- `Aborted` -> `"AbortedError: the update was not confirmed"`
- `NotFound { url }` -> `"NotFoundError: no resource found at {url} (HTTP 404)"`
- `Unauthorized { status, url }` -> `"UnauthorizedError: request to {url} was not authorized (HTTP {status})"`
- `HttpStatus { status, url }` -> `"HttpStatusError: request to {url} failed with status {status}"`
- `Release(s)` -> `"ReleaseError: {s}"`
- `Config(s)` -> `"ConfigError: {s}"`
- `Io(e)` -> `"IoError: {e}"`
- `Json(e)` -> `"JsonError: {e}"` (dereferences the box)
- `Transport(e)` -> `"TransportError: {e}"` (dereferences the box)
- `SemVer(e)` -> `"SemVerError: {e}"` (dereferences the box)
- `Zip(e)` -> `"ZipError: {e}"` (dereferences the box, `archive-zip`)
- `ArchiveNotEnabled(s)` -> `"ArchiveNotEnabledError: Archive extension '{s}' not supported, please enable 'archive-{s}' feature!"`
- `NoSignatures(kind)` -> `"SignatureError: signature verification is only implemented for \`.tar.gz\` and \`.zip\` assets, not {kind} files"` (`signatures`)
- `Signature(e)` -> `"SignatureError: {e}"` (dereferences the box, `signatures`)
- `SignatureNonUTF8` -> `"SignatureError: cannot verify signature of a file with a non-UTF-8 name"` (`signatures`)
- `S3Auth(e)` -> `"S3AuthError: {e}"` (dereferences the box, `s3-auth`)

Note: `ArchiveNotEnabled` was corrected from `"ArchiveNotEnabled: ..."` to `"ArchiveNotEnabledError: ..."`;
`SignatureNonUTF8` was corrected from the bare message to `"SignatureError: ..."`, consistent with
every other variant using a `<Name>Error:` prefix.

### source() and downcast

`source()` returns the inner error for the wrapping variants: `Io` (the concrete io error), and the
boxed `Json`, `Transport`, `SemVer`, `Zip` (gated), `Signature` (gated), `S3Auth` (gated) -- each
via `&**e` to deref the box. All other variants (`Update`, `ChecksumMismatch`, `Aborted`,
`NotFound`, `Unauthorized`, `HttpStatus`, `Release`, `Config`, `ArchiveNotEnabled`, `NoSignatures`,
`SignatureNonUTF8`) return `None`. The concrete inner error of a boxed variant is reachable at
runtime through `source()` and `downcast_ref::<ConcreteType>()` (e.g.
`err.source().and_then(|s| s.downcast_ref::<reqwest::Error>())`).

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

`Transport`, `S3Auth`, `Zip`, `Signature`, `Json`, `SemVer` wrap `Box<dyn std::error::Error + Send +
Sync>` so no dependency type appears in the public API. The inner type can change (reqwest vs ureq
selection, a `zip`/`serde_json`/`semver` major bump, the signing implementation) without altering
the public surface. Inspection is still possible via `source()` + downcast. (`Io` is the exception:
it carries the std type directly, since `std::io::Error` is stable std.)

## Public surface

- `pub enum Error` with the variants above; `#[non_exhaustive]`.
- `pub type Result<T> = std::result::Result<T, Error>;` (`errors.rs:8`).
- `pub fn http_status(&self) -> Option<u16>` inherent method on `Error`.
- `pub fn url(&self) -> Option<&str>` inherent method on `Error`.
- Trait impls: `Debug` (derived), `Display`, `std::error::Error` (with `source()`).
- `From` impls: `std::io::Error`, `serde_json::Error`, `semver::Error` (always); `reqwest::Error`
  (`reqwest`), `ureq::Error` (`ureq`), `ZipError` (`archive-zip`), `ZipsignError` (`signatures`);
  and for `s3-auth`: `SystemTimeError`, `hmac::digest::InvalidLength`, `url::ParseError`,
  `time::error::ComponentRange`.
- The `bail!` / `format_err!` macros (`macros.rs`) build the String-carrying variants.
- `pub(crate) fn status_to_error(status: u16, url: &str) -> Error` (`errors.rs`) maps a status
  code to `NotFound` / `Unauthorized` / `HttpStatus`.

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
- A checksum digest mismatch produces `Error::ChecksumMismatch { expected, computed }` (not
  `Error::Update`). Both fields are lowercase hex-encoded digests.
- A user-declined confirmation prompt produces `Error::Aborted` (not `Error::Update`).
- `Error::Update` is reserved for genuine internal/programming failures: extractor invariants,
  archive-path failures, tokio blocking-task panics, verify-callback aborts.
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

Variant-routing is asserted across the backends: `Config` from invalid headers / missing fields
(`common.rs`, `github.rs`, `gitlab.rs`, `gitea.rs`, `s3.rs`), `Release` from
missing/empty/non-array payloads, `NotFound`/`Unauthorized`/`HttpStatus` on non-2xx (both clients
now produce the same variant, asserted in `github.rs`, `gitlab.rs`, `s3.rs`), and `HttpStatus`
propagation through pagination/retry (`backends/mod.rs`).

## Related

- `error-variant-granularity.md`
- `1.0-api-surface.md`
- `ref-update-pipeline.md`

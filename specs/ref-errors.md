# Error model (reference)

Status: implemented

## Scope

The crate's single public error type `errors::Error` (re-exported as `self_update::errors::Error`),
its `Result<T>` alias, the `Display` / `std::error::Error` (`source()`) impls, the `From`
conversions, and the `http_status()` helper. Source of truth: `src/errors.rs`. Construction sites
are spread across the backends, the HTTP clients, the update pipeline, and the checksum module.

## Behavior

`Error` is declared `#[derive(Debug)] #[non_exhaustive] pub enum` at `errors.rs:13-15`. Every
variant, what produces it, and its feature gate:

| Variant | Produced by | Feature gate | Opaque/boxed? |
| --- | --- | --- | --- |
| `Update(String)` | Catch-most update failure. Checksum mismatch (`checksum.rs:69`); aborted update (`lib.rs:520`, `update.rs:875`); extractor source has no file name (`lib.rs:728,818`); extract failures (`lib.rs:837,866`); blocking-task join failure (`custom.rs:385,392,399`). | none | no (String) |
| `NotFound { url: String }` | A request completed and returned HTTP 404. Raised by both HTTP clients when the response status is 404. | none | no (struct fields) |
| `Unauthorized { status: u16, url: String }` | A request completed and returned HTTP 401 or 403. `status` holds the exact code. Raised by both HTTP clients. | none | no (struct fields) |
| `HttpStatus { status: u16, url: String }` | A request completed and returned any other non-2xx status (e.g. 500, 503). Raised by both HTTP clients. | none | no (struct fields) |
| `Release(String)` | Problem with the resolved release: no asset for target (`update.rs:722`), missing asset/release fields in each backend's `from_value` (`github.rs:22-44`, `gitlab.rs:22-44`, `gitea.rs:21-43`), no releases found, empty/non-array payloads, missing tag (`gitlab.rs`, `gitea.rs`, `s3.rs:364,395`, `update.rs:1282`), S3 XML/regex parse failure (`s3.rs:766,835`). | none | no (String) |
| `Config(String)` | Builder/configuration error: missing required field (`update.rs:183`, `backends/common.rs:156-169`, `custom.rs:179,289`), missing `repo_owner`/`repo_name`/`bucket_name`/`region` (`github.rs`, `gitlab.rs`, `gitea.rs`, `s3.rs`), invalid HTTP header name/value (`lib.rs:1286,1289`, surfaced from `build()` via `common.rs:77`), auth-token parse failure (`github.rs:494`, `gitlab.rs:494`, `gitea.rs:498`), host-extraction failure (`s3.rs:555`). | none | no (String) |
| `Io(std::io::Error)` | Wraps a `std::io::Error`. Constructed directly (`lib.rs:721,767,812`) and via `From<std::io::Error>`. | none | no (concrete `std::io::Error`) |
| `Json(Box<dyn Error + Send + Sync>)` | `serde_json` failure, only via `From<serde_json::Error>` (`errors.rs:180-184`). | none | yes (boxed) |
| `Transport(Box<dyn Error + Send + Sync>)` | The request could not be completed (connection/TLS/timeout/transport failure). Only via `From<reqwest::Error>` (`reqwest` feature) or `From<ureq::Error>` (`ureq` feature) (`errors.rs:186-197`). A bare `?` on a client call lands here only when the error is not a status-code error. | none for the variant; the `From` impls are gated on `reqwest` / `ureq` | yes (boxed) |
| `SemVer(Box<dyn Error + Send + Sync>)` | `semver` parse failure, only via `From<semver::Error>` (`errors.rs:199-203`). | none | yes (boxed) |
| `Zip(Box<dyn Error + Send + Sync>)` | `zip` archive error, only via `From<ZipError>` (`errors.rs:205-210`). | `archive-zip` | yes (boxed) |
| `ArchiveNotEnabled(String)` | Archive extension whose `archive-*` feature is not enabled (`lib.rs:602,613,624,640`). String is the extension (`"zip"`/`"tar"`). | none | no (String) |
| `NoSignatures(crate::ArchiveKind)` | Archive contains no signatures to verify (`update.rs:946`). | `signatures` | no (carries `ArchiveKind`) |
| `Signature(Box<dyn Error + Send + Sync>)` | Signature-verification failure, only via `From<ZipsignError>` (`errors.rs:212-217`). | `signatures` | yes (boxed) |
| `SignatureNonUTF8` | Generated archive path contains non-UTF-8 characters so its signature cannot be verified (`update.rs:922`). Unit variant. | `signatures` | no (unit) |
| `S3Auth(Box<dyn Error + Send + Sync>)` | S3 SigV4 request-signing failure. Via `From<SystemTimeError>`, `From<hmac::digest::InvalidLength>`, `From<url::ParseError>`, `From<time::error::ComponentRange>` (`errors.rs:219-245`). | `s3-auth` | yes (boxed) |

### Display (`errors.rs:115-154`)

Each variant renders with a specific Display string:

- `Update(s)` -> `"UpdateError: {s}"`
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
- `ArchiveNotEnabled(s)` -> `"ArchiveNotEnabled: Archive extension '{s}' not supported, please enable 'archive-{s}' feature!"`
- `NoSignatures(kind)` -> `"No signature verification implemented for {kind:?} files"` (`signatures`)
- `Signature(e)` -> `"SignatureError: {e}"` (dereferences the box, `signatures`)
- `SignatureNonUTF8` -> `"Cannot verify signature of a file with a non-UTF-8 name"` (`signatures`)
- `S3Auth(e)` -> `"S3AuthError: {e}"` (dereferences the box, `s3-auth`)

### source() and downcast (`errors.rs:156-172`)

`source()` returns the inner error for the wrapping variants: `Io` (the concrete io error), and the
boxed `Json`, `Transport`, `SemVer`, `Zip` (gated), `Signature` (gated), `S3Auth` (gated) -- each
via `&**e` to deref the box. All other variants (`Update`, `NotFound`, `Unauthorized`, `HttpStatus`,
`Release`, `Config`, `ArchiveNotEnabled`, `NoSignatures`, `SignatureNonUTF8`) return `None`. The
concrete inner error of a boxed variant is reachable at runtime through `source()` and
`downcast_ref::<ConcreteType>()` (e.g.
`err.source().and_then(|s| s.downcast_ref::<reqwest::Error>())`).

### http_status() helper (`errors.rs:102-113`)

```rust
pub fn http_status(&self) -> Option<u16>
```

Returns the HTTP status code when the error came from a completed non-2xx response:
- `NotFound { .. }` -> `Some(404)`
- `Unauthorized { status, .. }` -> `Some(status)`
- `HttpStatus { status, .. }` -> `Some(status)`
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
- `pub fn http_status(&self) -> Option<u16>` inherent method on `Error` (`errors.rs:105`).
- Trait impls: `Debug` (derived), `Display`, `std::error::Error` (with `source()`).
- `From` impls: `std::io::Error`, `serde_json::Error`, `semver::Error` (always); `reqwest::Error`
  (`reqwest`), `ureq::Error` (`ureq`), `ZipError` (`archive-zip`), `ZipsignError` (`signatures`);
  and for `s3-auth`: `SystemTimeError`, `hmac::digest::InvalidLength`, `url::ParseError`,
  `time::error::ComponentRange`.
- The `bail!` / `format_err!` macros (`macros.rs:508-525`) build the String-carrying variants.
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
- The signatures-gated unit variant is named `SignatureNonUTF8` (renamed from `NonUTF8` in commit
  `4beb7f5`); its `Display` is "Cannot verify signature of a file with a non-UTF-8 name".

## Tests

`errors.rs` (`mod tests`): each boxed variant is asserted opaque-with-`source()` and its
`Display` prefix + embedded inner message (`Json`, `SemVer`, `Zip` gated, `Signature` gated);
`reqwest_error_maps_to_transport_variant` and `ureq_error_maps_to_transport_variant` pin the
`From<*::Error>` -> `Transport` mapping per client; `not_found_display_matches_spec`,
`unauthorized_display_matches_spec_401`, `unauthorized_display_matches_spec_403`,
`http_status_display_matches_spec` pin the exact Display strings; `http_status_helper_*` tests pin
`http_status()` return values; `status_to_error_*` tests pin the 404/401/403/500/503 mapping;
`signature_non_utf8_variant_is_renamed_and_displays` pins the rename and message.

Variant-routing is asserted across the backends: `Config` from invalid headers / missing fields
(`common.rs`, `github.rs`, `gitlab.rs`, `gitea.rs`, `s3.rs`), `Release` from
missing/empty/non-array payloads, `NotFound`/`Unauthorized`/`HttpStatus` on non-2xx (both clients
now produce the same variant, asserted in `github.rs`, `gitlab.rs`, `s3.rs`), and `HttpStatus`
propagation through pagination/retry (`backends/mod.rs`).

## Related

- `error-variant-granularity.md`
- `1.0-api-surface.md`
- `ref-update-pipeline.md`

# Error model (reference)

Status: implemented

## Scope

The crate's single public error type `errors::Error` (re-exported as `self_update::errors::Error`),
its `Result<T>` alias, the `Display` / `std::error::Error` (`source()`) impls, and the `From`
conversions. Source of truth: `src/errors.rs`. Construction sites are spread across the backends,
the HTTP clients, the update pipeline, and the checksum module.

## Behavior

`Error` is declared `#[derive(Debug)] #[non_exhaustive] pub enum` at `errors.rs:13-15`. Every
variant, what produces it, and its feature gate:

| Variant | Produced by | Feature gate | Opaque/boxed? |
| --- | --- | --- | --- |
| `Update(String)` | Catch-most update failure. Checksum mismatch (`checksum.rs:69`); aborted update (`lib.rs:520`, `update.rs:875`); extractor source has no file name (`lib.rs:728,818`); extract failures (`lib.rs:837,866`); blocking-task join failure (`custom.rs:385,392,399`). | none | no (String) |
| `Network(String)` | A request that completed but returned a non-success HTTP status. Raised by the reqwest and ureq clients after `!status().is_success()` (`http_client/reqwest.rs:39,84`, `http_client/ureq.rs:50`). | none | no (String) |
| `Release(String)` | Problem with the resolved release: no asset for target (`update.rs:722`), missing asset/release fields in each backend's `from_value` (`github.rs:22-44`, `gitlab.rs:22-44`, `gitea.rs:21-43`), no releases found, empty/non-array payloads, missing tag (`gitlab.rs`, `gitea.rs`, `s3.rs:364,395`, `update.rs:1282`), S3 XML/regex parse failure (`s3.rs:766,835`). | none | no (String) |
| `Config(String)` | Builder/configuration error: missing required field (`update.rs:183`, `backends/common.rs:156-169`, `custom.rs:179,289`), missing `repo_owner`/`repo_name`/`bucket_name`/`region` (`github.rs`, `gitlab.rs`, `gitea.rs`, `s3.rs`), invalid HTTP header name/value (`lib.rs:1286,1289`, surfaced from `build()` via `common.rs:77`), auth-token parse failure (`github.rs:494`, `gitlab.rs:494`, `gitea.rs:498`), host-extraction failure (`s3.rs:555`). | none | no (String) |
| `Io(std::io::Error)` | Wraps a `std::io::Error`. Constructed directly (`lib.rs:721,767,812`) and via `From<std::io::Error>`. | none | no (concrete `std::io::Error`) |
| `Json(Box<dyn Error + Send + Sync>)` | `serde_json` failure, only via `From<serde_json::Error>` (`errors.rs:134-138`). | none | yes (boxed) |
| `Http(Box<dyn Error + Send + Sync>)` | Transport / client-level failure from the active HTTP client. Only via `From<reqwest::Error>` (`reqwest` feature) or `From<ureq::Error>` (`ureq` feature) (`errors.rs:140-152`). A bare `?` on a client call (e.g. ureq `call()?` in `s3.rs`) lands here. | none for the variant; the `From` impls are gated on `reqwest` / `ureq` | yes (boxed) |
| `SemVer(Box<dyn Error + Send + Sync>)` | `semver` parse failure, only via `From<semver::Error>` (`errors.rs:154-158`). | none | yes (boxed) |
| `Zip(Box<dyn Error + Send + Sync>)` | `zip` archive error, only via `From<ZipError>` (`errors.rs:160-165`). | `archive-zip` | yes (boxed) |
| `ArchiveNotEnabled(String)` | Archive extension whose `archive-*` feature is not enabled (`lib.rs:602,613,624,640`). String is the extension (`"zip"`/`"tar"`). | none | no (String) |
| `NoSignatures(crate::ArchiveKind)` | Archive contains no signatures to verify (`update.rs:946`). | `signatures` | no (carries `ArchiveKind`) |
| `Signature(Box<dyn Error + Send + Sync>)` | Signature-verification failure, only via `From<ZipsignError>` (`errors.rs:167-172`). | `signatures` | yes (boxed) |
| `SignatureNonUTF8` | Generated archive path contains non-UTF-8 characters so its signature cannot be verified (`update.rs:922`). Unit variant. | `signatures` | no (unit) |
| `S3Auth(Box<dyn Error + Send + Sync>)` | S3 SigV4 request-signing failure. Via `From<SystemTimeError>`, `From<hmac::digest::InvalidLength>`, `From<url::ParseError>`, `From<time::error::ComponentRange>` (`errors.rs:174-200`). | `s3-auth` | yes (boxed) |

### Display (`errors.rs:79-108`)

Each variant renders with a `<Name>Error: <inner>` style prefix (e.g. `NetworkError:`,
`HttpError:`, `JsonError:`, `ZipError:`, `S3AuthError:`). The boxed variants deref the box so the
inner error's own `Display` is embedded (pinned by tests). `ArchiveNotEnabled`, `NoSignatures`, and
`SignatureNonUTF8` render bespoke messages.

### source() and downcast (`errors.rs:110-126`)

`source()` returns the inner error for the wrapping variants: `Io` (the concrete io error), and the
boxed `Json`, `Http`, `SemVer`, `Zip` (gated), `Signature` (gated), `S3Auth` (gated) -- each via
`&**e` to deref the box. All other variants (`Update`, `Network`, `Release`, `Config`,
`ArchiveNotEnabled`, `NoSignatures`, `SignatureNonUTF8`) return `None`. The concrete inner error of
a boxed variant is reachable at runtime through `source()` and `downcast_ref::<ConcreteType>()`
(e.g. `err.source().and_then(|s| s.downcast_ref::<reqwest::Error>())`).

### Why boxed

`Http`, `S3Auth`, `Zip`, `Signature`, `Json`, `SemVer` wrap `Box<dyn std::error::Error + Send +
Sync>` so no dependency type appears in the public API. The inner type can change (reqwest vs ureq
selection, a `zip`/`serde_json`/`semver` major bump, the signing implementation) without altering
the public surface. Inspection is still possible via `source()` + downcast. (`Io` is the exception:
it carries the std type directly, since `std::io::Error` is stable std.)

### Network vs Http

Both are HTTP-related but distinct. `Network(String)` is raised by the crate when a request
completes with a non-2xx status (the `!status().is_success()` `bail!` in both clients). `Http(box)`
is a transport/client error surfaced through the `From<reqwest::Error>` / `From<ureq::Error>`
conversion on a bare `?`. The two clients can diverge on the same condition: in the S3 listing
path, reqwest reaches the explicit `is_success()` check and yields `Network`, while ureq's
`call()?` short-circuits on its own status error and yields `Http` (pinned at `s3.rs:1272-1284`).
The naming is acknowledged as counterintuitive; see `error-network-vs-http-semantics.md`.

## Public surface

- `pub enum Error` with the variants above; `#[non_exhaustive]`.
- `pub type Result<T> = std::result::Result<T, Error>;` (`errors.rs:8`).
- Trait impls: `Debug` (derived), `Display`, `std::error::Error` (with `source()`).
- `From` impls: `std::io::Error`, `serde_json::Error`, `semver::Error` (always); `reqwest::Error`
  (`reqwest`), `ureq::Error` (`ureq`), `ZipError` (`archive-zip`), `ZipsignError` (`signatures`);
  and for `s3-auth`: `SystemTimeError`, `hmac::digest::InvalidLength`, `url::ParseError`,
  `time::error::ComponentRange`.
- The `bail!` / `format_err!` macros (`macros.rs:508-525`) build the String-carrying variants.

## Invariants and regression checklist

- `Error` is `#[non_exhaustive]`: downstream `match` must include a wildcard arm; new variants are
  not a breaking change.
- The opaque variants (`Json`, `Http`, `SemVer`, `Zip`, `Signature`, `S3Auth`) expose their inner
  error via `source()` (deref of the box), and `Display` embeds the inner message with the
  `<Name>Error:` prefix.
- No public dependency type leaks: the wrapping variants are `Box<dyn Error + Send + Sync>`, never a
  concrete `reqwest` / `ureq` / `zip` / `serde_json` / `semver` / `zipsign` type. `Io` deliberately
  carries the std `io::Error`.
- `Network` = completed request with non-success status; `Http` = transport/client `?` failure. Do
  not collapse them.
- The signatures-gated unit variant is named `SignatureNonUTF8` (renamed from `NonUTF8` in commit
  `4beb7f5`); its `Display` is "Cannot verify signature of a file with a non-UTF-8 name".

## Tests

`errors.rs:202-377` (`mod tests`): each boxed variant is asserted opaque-with-`source()` and its
`Display` prefix + embedded inner message (`Json`, `SemVer`, `Zip` gated, `Signature` gated);
`signature_non_utf8_variant_is_renamed_and_displays` pins the rename and message. Variant-routing
is asserted across the backends: `Config` from invalid headers / missing fields (`common.rs`,
`github.rs`, `gitlab.rs`, `gitea.rs`, `s3.rs`), `Release` from missing/empty/non-array payloads,
`Network` vs `Http` on non-2xx (`s3.rs:1272-1284`, `gitlab.rs`), and `Network` propagation through
pagination/retry (`backends/mod.rs`).

## Related

- `error-variant-granularity.md`
- `error-network-vs-http-semantics.md`
- `1.0-api-surface.md`
- `ref-update-pipeline.md`

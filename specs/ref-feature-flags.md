# Feature flags (reference)

Status: implemented

## Scope

Documents every Cargo feature of the `self_update` crate, the dependencies and
public API surface each feature gates, the feature-to-feature implication graph,
and the client/TLS selection rules. `reqwest` and `ureq` may coexist (the crate
picks `reqwest` by default and lets a caller inject any `HttpClient`), and so may
`native-tls` and `rustls` (`rustls` wins); only a no-client build or `async`
without `reqwest` is a `compile_error!`. Source of truth: the `[features]` table in
`Cargo.toml` (lines 67-92), the `compile_error!` guards in `src/lib.rs` and
`src/http_client/mod.rs`, and the `#[cfg(feature = ...)]` sites across `src/`. The S3
backend is gated behind the `s3` feature; `s3-auth` (private-bucket request
signing) implies `s3`.

## Behavior

All features and their wiring (`Cargo.toml:67-91`):

| Feature | Enables (deps / sub-features) | Implies | Notes |
|---------|------------------------------|---------|-------|
| `default` | `reqwest`, `rustls`, `progress-bar`, `github` | client + TLS + progress + backend | the default feature set (`Cargo.toml:68`) |
| `reqwest` | `dep:reqwest` (blocking, json, http2) | an HTTP client | may coexist with `ureq`; `reqwest` is the default-picked client when both are on (`Cargo.toml:86`) |
| `ureq` | `dep:ureq` (gzip, json, socks-proxy, charset) | an HTTP client | to use `ureq` alone, set `--no-default-features` so `reqwest` (a default) is not pulled (`Cargo.toml:87`) |
| `native-tls` | `reqwest?/native-tls`, `ureq?/native-tls` | a TLS backend | forwards native-TLS to whichever client is on (`Cargo.toml:83`) |
| `rustls` | `reqwest?/rustls`, `ureq?/rustls` | a TLS backend | may coexist with `native-tls`; `rustls` wins when both are on (`Cargo.toml:84`) |
| `async` | `reqwest`, `reqwest?/stream`, `dep:tokio`, `dep:futures-util` | `reqwest` | reqwest-only; incompatible with `ureq` (`Cargo.toml:80`) |
| `archive-zip` | `zip`, `zipsign-api?/verify-zip` | - | enables zip extraction; wires zip signature verify when `signatures` on (`Cargo.toml:70`) |
| `archive-tar` | `tar`, `zipsign-api?/verify-tar` | - | enables tar extraction; wires tar signature verify when `signatures` on (`Cargo.toml:73`) |
| `compression-zip-bzip2` | `zip/bzip2` | `archive-zip` | bzip2 inside zip (`Cargo.toml:71`) |
| `compression-zip-deflate` | `zip/deflate` | `archive-zip` | deflate inside zip (`Cargo.toml:72`) |
| `compression-tar-gz` | `flate2`, `either` | `archive-tar` | gzip; selects `Either<File, GzDecoder>` reader type (`Cargo.toml:74`, `lib.rs:665-668`) |
| `progress-bar` | `dep:indicatif` | - | terminal progress bar in `Download`; the `progress_callback` byte hook is always-on and not gated (`Cargo.toml:77`) |
| `signatures` | `dep:zipsign-api` | - | ed25519ph verify; `verify-zip`/`verify-tar` come from the archive features (`Cargo.toml:75`) |
| `checksums` | `dep:sha2` | - | sha2 checksum verify (`Cargo.toml:76`) |
| `github` | `dep:...` (github backend module) | - | gates the GitHub backend; default-on (`Cargo.toml:88`) |
| `gitlab` | `dep:...` (gitlab backend module) | - | gates the GitLab backend; off by default (`Cargo.toml:89`) |
| `gitea` | `dep:...` (gitea backend module) | - | gates the Gitea backend; off by default (`Cargo.toml:90`) |
| `s3` | `dep:quick-xml` (s3 backend module) | - | gates the S3 backend and the `quick-xml` dependency; off by default (`Cargo.toml:91`) |
| `s3-auth` | `dep:hmac`, `dep:percent-encoding`, `dep:sha2`, `dep:url`, `dep:time` | `s3` | SigV4 request signing for private buckets; implies `s3` (`Cargo.toml:92`) |

Implication notes:

- `archive-zip` implies `zip`; the `compression-zip-*` features imply
  `archive-zip` and add a codec to the `zip` dep.
- `archive-tar` implies `tar`; `compression-tar-gz` implies `archive-tar` and
  adds `flate2` + `either` (gzip is the only `Compression` variant,
  `lib.rs:582-586`).
- `signatures` only pulls `zipsign-api` (`dep:zipsign-api`). The actual
  `verify-zip` / `verify-tar` sub-features are pulled in by `archive-zip` /
  `archive-tar` via the optional `zipsign-api?/verify-*` syntax, so signature
  verification of a given archive kind is enabled only when both `signatures`
  and the matching archive feature are on (`Cargo.toml:70,73,75`;
  `update.rs:904-938`).
- `async` requires the `reqwest` client plus `tokio` and `futures-util`, and is
  incompatible with `ureq` (`Cargo.toml:80`, guard at `lib.rs:433-437`).

Client and TLS coexistence: `reqwest` and `ureq` may both be enabled, and so may
`native-tls` and `rustls`; `cargo build --all-features` builds. When more than one
client is on, `default_client()` picks `reqwest` (`http_client/mod.rs:98-106`); when
both TLS backends are on, the per-call client builders prefer `rustls`. An injected
`http_client(Arc<dyn HttpClient>)` overrides the default pick entirely.

The only `compile_error!` guards that remain:

- neither `reqwest` nor `ureq` -> error (`http_client/mod.rs:111`):
  "no HTTP client selected - enable at least one of the `reqwest` (default) or
  `ureq` features". A build with no client cannot service any request.
- `async` without `reqwest` -> error (`lib.rs:434`):
  "feature `async` requires the `reqwest` client - `ureq` has no async API".
  (`async` implies `reqwest` in `Cargo.toml`, so this only fires on a manual
  inconsistent feature set.)

MSRV / edition: `rust-version = "1.85"`, `edition = "2024"` (`Cargo.toml:12-13`).

docs.rs feature set (`Cargo.toml:17-29`): `reqwest`, `rustls`,
`archive-zip`, `compression-zip-bzip2`, `compression-zip-deflate`,
`archive-tar`, `compression-tar-gz`, `progress-bar`, `signatures`, `checksums`,
`github`, `gitlab`, `gitea`, `s3`, `s3-auth`, `async`. This pins the documented
client/TLS pair to `reqwest` + `rustls` for a stable rendered surface. The same
`[package.metadata.docs.rs]` block also sets `rustdoc-args = ["--cfg", "docsrs"]`
(`Cargo.toml:30`), which sets the `docsrs` cfg so the crate enables the nightly
`doc_cfg` feature (`lib.rs:410`) and the gated public re-exports carry
`#[cfg_attr(docsrs, doc(cfg(...)))]`; docs.rs then renders a feature-gate badge
on each gated item.

## Public surface

Feature-gated public items:

- `reqwest`: re-export `pub use reqwest` (`lib.rs:442-444`) and the
  `reqwest_client()` builder setter (`macros.rs:64-68`).
- `ureq`: re-export `pub use ureq` (`lib.rs:452-454`) and the `ureq_agent()`
  builder setter (`macros.rs:82-86`).
- `async`: re-export `pub use update::AsyncReleaseSource` (`lib.rs:445-447`),
  the `reqwest_async_client()` setter (`macros.rs:72-76`), and the `*_async`
  verbs across the backends (e.g. `github.rs`, `gitlab.rs`, `gitea.rs`,
  `s3.rs`, `custom.rs`, `update.rs`).
- `signatures`: re-export `pub use zipsign_api` and the
  `pub type VerifyingKey = [u8; zipsign_api::PUBLIC_KEY_LENGTH]` alias
  (`lib.rs:460-470`), plus the `verify_keys` builder setter (`macros.rs:455`)
  and accessor (`macros.rs:195-197`).
- `checksums`: `pub use checksum::Checksum` (`lib.rs:498-500`) and the
  `verify_checksum` builder setter (`macros.rs:438-442`) and accessor
  (`macros.rs:191`).
- All the gated crate-root re-exports above carry
  `#[cfg_attr(docsrs, doc(cfg(feature = "...")))]`, so docs.rs renders a
  feature-gate badge on each (`lib.rs:443,446,453,461,469,499`).
- `archive-tar`: `ArchiveKind::Tar` enum variant (`lib.rs:589-591`).
- `archive-zip`: `ArchiveKind::Zip` enum variant (`lib.rs:595-597`).
- `s3`: gates the S3 backend module (`backends/s3.rs`); also pulled in by `s3-auth`.
- `s3-auth`: the SigV4 signing path and credential/region builder surface in
  `backends/s3.rs` (e.g. `s3.rs:25,76,120,...`); implies `s3`.

`ArchiveKind` and `Extract` are public unconditionally, but `ArchiveKind` is
`#[non_exhaustive]` and its `Tar`/`Zip` variants only exist under their archive
features (`lib.rs:572-580`). `ArchiveKind` implements `Display` (`lib.rs:589`),
rendering `tar.gz` / `zip` / `tar` / `gz` / `plain`; the `Error::NoSignatures`
message uses it instead of the `Debug` form. `detect_archive` returns
`Error::ArchiveNotEnabled` for an extension whose archive feature is off
(`lib.rs:600-603,611-614`).

## Invariants and regression checklist

- At least one HTTP client must be enabled: a build with neither `reqwest` nor
  `ureq` is a `compile_error!` (`http_client/mod.rs:111`). Both may be on at once;
  `default_client()` picks `reqwest` (`http_client/mod.rs:98-106`).
- `native-tls` and `rustls` may both be on; the per-call client builders prefer
  `rustls` when both are set. Neither combination is a `compile_error!`.
- `async` requires `reqwest`: a build with `async` but not `reqwest` is a
  `compile_error!` (`lib.rs:434`). `async` force-enables `reqwest` (`Cargo.toml:91`),
  so this only fires on a hand-edited inconsistent feature set.
- `cargo build --all-features` MUST build: with both clients and both TLS backends
  coexisting, the only remaining guards are the no-client and async-without-reqwest
  cases, neither of which `--all-features` triggers. `make ci` builds the
  `--all-features` lane explicitly.
- The s3 backend is gated behind the `s3` feature; `s3-auth` implies `s3` so
  enabling `s3-auth` is sufficient for private-bucket request signing
  (`Cargo.toml:91-92`).
- A signed archive of a given kind verifies only when both `signatures` and the
  matching `archive-*` feature are enabled (the `verify-*` sub-feature rides on
  the archive feature, `Cargo.toml:69,72,74`).
- MSRV is 1.85; do not use APIs newer than that. Edition is 2024.

## Tests

CI (`.github/workflows/build.yml`) runs `make tests`, which builds and tests
four single-client feature sets (`Makefile` `tests` target):

- `tests/default`: `cargo test` (default = reqwest + rustls + progress-bar + github).
- `tests/reqwest`: full optional set on reqwest (`REQWEST_FEATURES` =
  archives + compression + signatures + checksums + s3-auth).
- `tests/ureq`: `--no-default-features --features "ureq native-tls
  <archive set>"`.
- `tests/async`: `async` + the full reqwest optional set.

`make check/clippy` runs clippy separately for reqwest, ureq, and async because
the clients cannot be combined. `make examples` builds each backend example
(github, gitlab, gitea, s3, custom) with `REQWEST_FEATURES`. There is no lane
that builds `--all-features` (by design it cannot compile).

## Related

- `ref-http-client.md` - the reqwest/ureq client abstraction these features select.
- `async-api.md` - the `*_async` verbs gated by the `async` feature.
- `checksum-verification.md`, `post-update-verify.md` - `checksums` / `signatures` verify paths.
- `ref-update-pipeline.md` - archive detection and extraction gated by `archive-*` / `compression-*`.

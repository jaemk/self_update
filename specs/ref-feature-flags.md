# Feature flags (reference)

Status: implemented

## Scope

Documents every Cargo feature of the `self_update` crate, the dependencies and
public API surface each feature gates, the feature-to-feature implication graph,
and the compile-time mutual-exclusion guards that enforce exactly one HTTP
client and exactly one TLS backend. Source of truth: the `[features]` table in
`Cargo.toml` (lines 66-92), the `compile_error!` guards in `src/lib.rs`
(lines 412-437), and the `#[cfg(feature = ...)]` sites across `src/`.

## Behavior

All features and their wiring (`Cargo.toml:66-92`):

| Feature | Enables (deps / sub-features) | Implies | Notes |
|---------|------------------------------|---------|-------|
| `default` | `reqwest`, `default-tls` | client + TLS | the default client/TLS pair (`Cargo.toml:67`) |
| `reqwest` | `dep:reqwest` (blocking, json, http2) | one HTTP client | mutually exclusive with `ureq` (`Cargo.toml:85`) |
| `ureq` | `dep:ureq` (gzip, json, socks-proxy, charset) | one HTTP client | requires `--no-default-features` to avoid pulling `reqwest` (`Cargo.toml:86`) |
| `default-tls` | `reqwest?/native-tls`, `ureq?/native-tls` | one TLS backend | forwards native-TLS to whichever client is on (`Cargo.toml:82`) |
| `rustls` | `reqwest?/rustls`, `ureq?/rustls` | one TLS backend | mutually exclusive with `default-tls` (`Cargo.toml:83`) |
| `async` | `reqwest`, `reqwest?/stream`, `dep:tokio`, `dep:futures-util` | `reqwest` | reqwest-only; incompatible with `ureq` (`Cargo.toml:79`) |
| `archive-zip` | `zip`, `zipsign-api?/verify-zip` | - | enables zip extraction; wires zip signature verify when `signatures` on (`Cargo.toml:69`) |
| `archive-tar` | `tar`, `zipsign-api?/verify-tar` | - | enables tar extraction; wires tar signature verify when `signatures` on (`Cargo.toml:72`) |
| `compression-zip-bzip2` | `zip/bzip2` | `archive-zip` | bzip2 inside zip (`Cargo.toml:70`) |
| `compression-zip-deflate` | `zip/deflate` | `archive-zip` | deflate inside zip (`Cargo.toml:71`) |
| `compression-flate2` | `flate2`, `either` | `archive-tar` | gzip; selects `Either<File, GzDecoder>` reader type (`Cargo.toml:73`, `lib.rs:665-668`) |
| `signatures` | `dep:zipsign-api` | - | ed25519ph verify; `verify-zip`/`verify-tar` come from the archive features (`Cargo.toml:74`) |
| `checksums` | `dep:sha2` | - | sha2 checksum verify (`Cargo.toml:75`) |
| `s3-auth` | `dep:hmac`, `dep:percent-encoding`, `dep:sha2`, `dep:url`, `dep:time` | - | SigV4 request signing for private buckets (`Cargo.toml:87`) |
| `s3` | (empty) | - | no-op alias; the s3 backend is always compiled (`Cargo.toml:89-92`) |

Implication notes:

- `archive-zip` implies `zip`; the `compression-zip-*` features imply
  `archive-zip` and add a codec to the `zip` dep.
- `archive-tar` implies `tar`; `compression-flate2` implies `archive-tar` and
  adds `flate2` + `either` (gzip is the only `Compression` variant,
  `lib.rs:582-586`).
- `signatures` only pulls `zipsign-api` (`dep:zipsign-api`). The actual
  `verify-zip` / `verify-tar` sub-features are pulled in by `archive-zip` /
  `archive-tar` via the optional `zipsign-api?/verify-*` syntax, so signature
  verification of a given archive kind is enabled only when both `signatures`
  and the matching archive feature are on (`Cargo.toml:69,72,74`;
  `update.rs:904-938`).
- `async` requires the `reqwest` client plus `tokio` and `futures-util`, and is
  incompatible with `ureq` (`Cargo.toml:79`, guard at `lib.rs:433-437`).

Mutual-exclusion guards (`src/lib.rs`), each a `compile_error!`:

- `reqwest` AND `ureq` both on -> error (`lib.rs:414-418`):
  "features `reqwest` and `ureq` are mutually exclusive - enable exactly one
  HTTP client (for `ureq`, set `default-features = false`)".
- neither `reqwest` nor `ureq` -> error (`lib.rs:419-422`):
  "no HTTP client selected - enable exactly one of the `reqwest` (default) or
  `ureq` features".
- `default-tls` AND `rustls` both on -> error (`lib.rs:426-430`):
  "features `default-tls` and `rustls` are mutually exclusive - to use
  `rustls`, set `default-features = false`".
- `async` AND `ureq` both on -> error (`lib.rs:433-437`):
  "feature `async` requires the `reqwest` client and is incompatible with
  `ureq` - `ureq` has no async API".

MSRV / edition: `rust-version = "1.85"`, `edition = "2018"` (`Cargo.toml:12-13`).

docs.rs feature set (`Cargo.toml:16-29`): `reqwest`, `default-tls`,
`archive-zip`, `compression-zip-bzip2`, `compression-zip-deflate`,
`archive-tar`, `compression-flate2`, `signatures`, `checksums`, `s3-auth`,
`async`. This is a single client (`reqwest`) + single TLS (`default-tls`) set
because `--all-features` does not build (see invariants).

## Public surface

Feature-gated public items:

- `reqwest`: re-export `pub use reqwest` (`lib.rs:442-443`) and the
  `reqwest_client()` builder setter (`macros.rs:64-68`).
- `ureq`: re-export `pub use ureq` (`lib.rs:450-451`) and the `ureq_agent()`
  builder setter (`macros.rs:82-86`).
- `async`: re-export `pub use update::AsyncReleaseSource` (`lib.rs:444-445`),
  the `reqwest_async_client()` setter (`macros.rs:72-76`), and the `*_async`
  verbs across the backends (e.g. `github.rs`, `gitlab.rs`, `gitea.rs`,
  `s3.rs`, `custom.rs`, `update.rs`).
- `signatures`: re-export `pub use zipsign_api` and the
  `pub type VerifyingKey = [u8; zipsign_api::PUBLIC_KEY_LENGTH]` alias
  (`lib.rs:457-465`), plus the `verifying_keys` builder methods and accessor
  (`macros.rs:194-197`).
- `checksums`: `pub use checksum::Checksum` (`lib.rs:491-492`) and the
  `checksum()` accessor (`macros.rs:190-192`).
- `archive-tar`: `ArchiveKind::Tar` enum variant (`lib.rs:575-576`).
- `archive-zip`: `ArchiveKind::Zip` enum variant (`lib.rs:578-579`).
- `s3-auth`: the SigV4 signing path and credential/region builder surface in
  `backends/s3.rs` (e.g. `s3.rs:25,76,120,...`). The s3 backend type itself is
  always compiled regardless of feature.

`ArchiveKind` and `Extract` are public unconditionally, but `ArchiveKind` is
`#[non_exhaustive]` and its `Tar`/`Zip` variants only exist under their archive
features (`lib.rs:572-580`). `detect_archive` returns
`Error::ArchiveNotEnabled` for an extension whose archive feature is off
(`lib.rs:600-603,611-614`).

## Invariants and regression checklist

- Exactly one HTTP client is enforced at compile time: enabling both `reqwest`
  and `ureq`, or neither, is a `compile_error!` (`lib.rs:414-422`).
- Exactly one TLS backend is enforced at compile time: enabling both
  `default-tls` and `rustls` is a `compile_error!` (`lib.rs:426-430`).
- `async` excludes `ureq`: enabling both is a `compile_error!`
  (`lib.rs:433-437`); `async` also force-enables `reqwest` (`Cargo.toml:79`).
- `cargo build --all-features` MUST NOT build: it would turn on `reqwest` +
  `ureq` (and `default-tls` + `rustls`) simultaneously, tripping the guards.
  Every build/test/clippy lane therefore selects exactly one client explicitly
  (`Makefile` header comment; `Cargo.toml:15`).
- `s3` is a no-op alias (empty feature list); `features = ["s3"]` resolves but
  enables nothing. Only private-bucket request signing needs `s3-auth`
  (`Cargo.toml:89-92`).
- A signed archive of a given kind verifies only when both `signatures` and the
  matching `archive-*` feature are enabled (the `verify-*` sub-feature rides on
  the archive feature, `Cargo.toml:69,72,74`).
- MSRV is 1.85; do not use APIs newer than that. Edition stays 2018.

## Tests

CI (`.github/workflows/build.yml`) runs `make tests`, which builds and tests
four single-client feature sets (`Makefile` `tests` target):

- `tests/default`: `cargo test` (default = reqwest + default-tls).
- `tests/reqwest`: full optional set on reqwest (`REQWEST_FEATURES` =
  archives + compression + signatures + checksums + s3-auth).
- `tests/ureq`: `--no-default-features --features "ureq default-tls
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

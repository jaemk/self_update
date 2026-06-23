# Error variant granularity

Status: implemented

## Problem

The catch-all stringly-typed variants `Error::Update(String)`,
`Error::Release(String)`, and `Error::Config(String)` carried only a message. A caller
could not match on them to handle distinct failure modes (for example, distinguishing a
missing release from an unparseable response, or a config error from the field that
caused it). Several construction sites also stringified and **discarded** a real
underlying error, so `Error::source()` returned `None` where a chain existed.

## Shipped

The HTTP-status part shipped in 1.0: `Error::Network(String)` was replaced by the
structured `Error::NotFound { url }`, `Error::Unauthorized { status, url }`, and
`Error::HttpStatus { status, url }`, with an `Error::http_status()` accessor.

The remaining `Update` / `Release` / `Config` string catch-alls are now structured. See `ref-errors.md` for the full variant table and the construction-site
mapping. Summary:

- `Config(String)` -> `MissingField { field: &'static str }` (required-field validation),
  `InvalidHeader { source }` (builder header validation), `InvalidAuthToken { source }`
  (auth-token encoding). A residual `Config(String)` is kept only for the `s3-auth`
  host-extraction site.
- `Release(String)` -> `NoReleaseFound { target: Option<String> }` (clean negative),
  `MissingAssetField { field: &'static str }` (missing payload field),
  `InvalidResponse { source }` (response parse failures: the S3 XML / regex sites whose
  source was previously discarded).
- `Update(String)` -> `VerificationRejected { reason: Option<String> }` (the user-controlled
  `verify_with` rejection) and `Internal { message, source }` (genuine invariant violations and
  blocking-task join failures, with the tokio `JoinError` carried as `source`).

The `source()`-chain breaks are fixed: the S3 regex build / XML parse, the
github/gitlab/gitea/`update.rs` auth-token parses, and the `custom.rs` / `update.rs`
`JoinError` sites now carry a boxed `source` and chain through `Error::source()`.

The new struct-form variants are `#[non_exhaustive]` so future fields stay non-breaking.

## Implementation

- Code: `src/errors.rs` (the enum, `Display`, `source()`, and the `MessageError` helper);
  construction sites in `src/lib.rs`, `src/update.rs`, `src/backends/common.rs`,
  `src/backends/{github,gitlab,gitea,s3,custom}.rs`.
- Tests: `src/errors.rs` (`mod tests`) unit tests per variant plus `source()` chaining,
  `Io` `ErrorKind` exposure, and a `#[non_exhaustive]` wildcard-match test; representative
  construction-site tests in `backends/common.rs` (`MissingField` / `InvalidHeader`),
  `backends/github.rs` (`InvalidAuthToken` source chain), `backends/s3.rs`
  (`InvalidResponse` XML-parse source chain), `backends/custom.rs` (`Internal` JoinError
  source chain), and `update.rs` (`VerificationRejected`).
- See the CHANGELOG `[unreleased]` entry.

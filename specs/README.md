# Specifications

This directory is the committed, canonical record of `self_update`'s feature
specifications. Each file states a status, the problem it addresses, and the
decision taken (or, for deferred work, what it would take and why it waits).

## Status legend

- `implemented` — shipped in the crate; the spec points at the code and CHANGELOG.
- `not implemented` — a decided-against-for-now item with a concrete additive path.
- `needs research` — a known gap whose shape is not yet settled.

## Index

| Spec | Status | Summary |
|------|--------|---------|
| [async-api.md](async-api.md) | implemented | Async update verbs behind the `async` feature (reqwest + tokio). |
| [transport-control.md](transport-control.md) | implemented | Timeouts, headers, retries, proxy, and injectable HTTP clients. |
| [release-scan-pagination.md](release-scan-pagination.md) | implemented | `update()` now paginates the release listing. |
| [custom-backends.md](custom-backends.md) | implemented | `ReleaseSource` / `AsyncReleaseSource` + `backends::custom`. |
| [checksum-verification.md](checksum-verification.md) | implemented | Caller-pinned digest check behind the `checksums` feature. |
| [multi-file-install.md](multi-file-install.md) | implemented | `MoveAll` transactional multi-file installer. |
| [custom-asset-matching.md](custom-asset-matching.md) | implemented | `asset_matcher` override for asset selection. |
| [progress-callback.md](progress-callback.md) | implemented | Byte-level `progress_callback` independent of indicatif. |
| [s3-auth-token-removal.md](s3-auth-token-removal.md) | implemented | Removed the no-op s3 `auth_token` setter. |
| [post-update-verify.md](post-update-verify.md) | implemented | `verify_with` hook on the extracted binary before swap. |
| [1.0-api-surface.md](1.0-api-surface.md) | implemented | The 1.0 ergonomic and naming surface that shipped together. |
| [releases-check-type.md](releases-check-type.md) | implemented | The fetch-once `Releases` check type and its ergonomics. |
| [error-variant-granularity.md](error-variant-granularity.md) | not implemented | Splitting the stringly-typed catch-all `Error` variants. |
| [s3-max-keys-configurable.md](s3-max-keys-configurable.md) | not implemented | A builder setter for the s3 `MAX_KEYS` per-request cap. |
| [update-config-internal-accessors.md](update-config-internal-accessors.md) | needs research | Moving crate-private-typed `UpdateConfig` accessors off the public trait. |
| [releases-test-constructor.md](releases-test-constructor.md) | needs research | A downstream-buildable `Releases` for unit tests. |
| [error-network-vs-http-semantics.md](error-network-vs-http-semantics.md) | not implemented | Clarifying `Error::Network` vs `Error::Http`. |
| [choose-latest-release-sort.md](choose-latest-release-sort.md) | needs research | A total-order comparator in `choose_latest_release`. |
| [async-future-extensions.md](async-future-extensions.md) | not implemented | Further async work beyond the current verbs. |
| [checksum-from-asset.md](checksum-from-asset.md) | not implemented | Fetching and parsing a `SHA256SUMS` asset for the digest. |

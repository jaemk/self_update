# Specifications

This directory is the committed, canonical record of `self_update`'s behavior and
design. It serves two roles:

- **Behavior reference** (`ref-*.md`) documents what each subsystem does today,
  cited to `file:line`. These are the source of truth for defining existing
  functionality, evaluating a proposed change, and detecting regressions: if code
  and a `ref-*` spec disagree, one of them is a bug.
- **Decisions and deferred work** records why the surface is shaped the way it is
  (implemented decisions) and what is intentionally left for later (deferred /
  needs-research items, each with a concrete additive path).

## How to use these specs

- Before changing a subsystem, read its `ref-*` spec to learn the current contract
  and the invariants a change must preserve (each reference spec ends with an
  "Invariants and regression checklist").
- When a change alters behavior, update the matching `ref-*` spec in the same change,
  the same way `README.md` and `CHANGELOG.md` are kept in sync.
- When adding functionality, write or extend the relevant `ref-*` spec to describe
  the new contract; if a decision is involved, add a decision spec.
- Deferred items move to `implemented` (and usually gain or update a `ref-*` spec)
  once shipped.

## Status legend

- `implemented` - shipped in the crate; the spec points at the code and CHANGELOG.
- `not implemented` - a decided-against-for-now item with a concrete additive path.
- `needs research` - a known gap whose shape is not yet settled.

## Behavior reference

Current behavior per subsystem. All `implemented`.

| Spec | Subsystem | Source |
|------|-----------|--------|
| [ref-update-pipeline.md](ref-update-pipeline.md) | The end-to-end download / verify / extract / replace flow, confirm + output, multi-file install | `src/update.rs`, `src/lib.rs` |
| [ref-release-model.md](ref-release-model.md) | `Release` / `ReleaseAsset` / `Releases` and the sealed `ReleaseUpdate` / `UpdateConfig` fetch traits | `src/update.rs` |
| [ref-version-and-target.md](ref-version-and-target.md) | Semver parsing and comparison, `get_target()`, asset matching and overrides | `src/version.rs`, `src/lib.rs` |
| [ref-common-config.md](ref-common-config.md) | `CommonBuilderConfig` / `CommonConfig` and the builder-setter / accessor / async-method macros | `src/backends/common.rs`, `src/macros.rs` |
| [ref-http-client.md](ref-http-client.md) | The reqwest/ureq abstraction, TLS, timeouts, retries, proxy, client injection | `src/http_client/`, `src/macros.rs`, `src/backends/mod.rs` |
| [ref-github-backend.md](ref-github-backend.md) | GitHub release listing, by-tag fetch, auth, pagination, JSON mapping | `src/backends/github.rs` |
| [ref-gitlab-backend.md](ref-gitlab-backend.md) | GitLab release listing, by-tag fetch, auth, project-path encoding | `src/backends/gitlab.rs` |
| [ref-gitea-backend.md](ref-gitea-backend.md) | Gitea release listing, by-tag fetch, auth, pagination | `src/backends/gitea.rs` |
| [ref-s3-backend.md](ref-s3-backend.md) | S3-compatible listing, URL composition, XML mapping, SigV4 signing | `src/backends/s3.rs` |
| [ref-custom-backend.md](ref-custom-backend.md) | `ReleaseSource` / `AsyncReleaseSource` traits and the `backends::custom` adapters | `src/backends/custom.rs`, `src/update.rs` |
| [ref-signatures-and-checksums.md](ref-signatures-and-checksums.md) | Artifact verification (`checksums` digest, `signatures` zipsign) and its pipeline ordering | `src/checksum.rs`, `src/update.rs` |
| [ref-errors.md](ref-errors.md) | Every `Error` variant, its producer, feature gate, and opaque boxing | `src/errors.rs` |
| [ref-feature-flags.md](ref-feature-flags.md) | Cargo features, what each gates, and the compile-time mutual-exclusion guards | `Cargo.toml`, `src/lib.rs` |

## Decisions and deferred work

### Implemented decisions

| Spec | Summary |
|------|---------|
| [1.0-api-surface.md](1.0-api-surface.md) | The 1.0 ergonomic and naming surface that shipped together. |
| [releases-check-type.md](releases-check-type.md) | The fetch-once `Releases` check type and its ergonomics. |
| [async-api.md](async-api.md) | Async update verbs behind the `async` feature (reqwest + tokio). |
| [transport-control.md](transport-control.md) | Timeouts, headers, retries, proxy, and injectable HTTP clients. |
| [release-scan-pagination.md](release-scan-pagination.md) | `update()` paginates the release listing. |
| [custom-backends.md](custom-backends.md) | `ReleaseSource` / `AsyncReleaseSource` + `backends::custom`. |
| [checksum-verification.md](checksum-verification.md) | Caller-pinned digest check behind the `checksums` feature. |
| [multi-file-install.md](multi-file-install.md) | `MoveAll` transactional multi-file installer. |
| [custom-asset-matching.md](custom-asset-matching.md) | `asset_matcher` override for asset selection. |
| [progress-callback.md](progress-callback.md) | Byte-level `progress_callback` independent of indicatif. |
| [s3-auth-token-removal.md](s3-auth-token-removal.md) | Removed the no-op s3 `auth_token` setter. |
| [post-update-verify.md](post-update-verify.md) | `verify_with` hook on the extracted binary before swap. |
| [release-tag-url-encoding.md](release-tag-url-encoding.md) | Percent-encoding the tag segment in the fetch-by-tag URLs. |

### Deferred (not implemented)

| Spec | Summary |
|------|---------|
| [error-variant-granularity.md](error-variant-granularity.md) | Splitting the stringly-typed catch-all `Error` variants. |
| [error-network-vs-http-semantics.md](error-network-vs-http-semantics.md) | Clarifying `Error::Network` vs `Error::Http`. |
| [s3-max-keys-configurable.md](s3-max-keys-configurable.md) | A builder setter for the s3 `MAX_KEYS` per-request cap. |
| [async-future-extensions.md](async-future-extensions.md) | Further async work beyond the current verbs. |
| [checksum-from-asset.md](checksum-from-asset.md) | Fetching and parsing a `SHA256SUMS` asset for the digest. |

### Needs research

| Spec | Summary |
|------|---------|
| [update-config-internal-accessors.md](update-config-internal-accessors.md) | Moving crate-private-typed `UpdateConfig` accessors off the public trait. |
| [releases-test-constructor.md](releases-test-constructor.md) | A downstream-buildable `Releases` for unit tests. |
| [choose-latest-release-sort.md](choose-latest-release-sort.md) | A total-order comparator in `choose_latest_release`. |

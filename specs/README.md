# Spec

Every feature is documented here before or as it lands, with its status.

## Feature status

Status values: `done` (implemented and covered by tests), `partial` (the core is
built and tested, with a documented sub-item still deferred), `pending`
(documented, not yet built; the default), `research` (needs investigation or
design before it can be built). Keep each row's status current with `spec.py set`.

| Feature | Status | Spec |
|---------|--------|------|
| Update Pipeline | done | [ref-update-pipeline.md](ref-update-pipeline.md) |
| Release Model | done | [ref-release-model.md](ref-release-model.md) |
| Version and Target | done | [ref-version-and-target.md](ref-version-and-target.md) |
| Common Config | done | [ref-common-config.md](ref-common-config.md) |
| HTTP Client | done | [ref-http-client.md](ref-http-client.md) |
| GitHub Backend | done | [ref-github-backend.md](ref-github-backend.md) |
| GitLab Backend | done | [ref-gitlab-backend.md](ref-gitlab-backend.md) |
| Gitea Backend | done | [ref-gitea-backend.md](ref-gitea-backend.md) |
| S3 Backend | done | [ref-s3-backend.md](ref-s3-backend.md) |
| Custom Backend | done | [ref-custom-backend.md](ref-custom-backend.md) |
| Signatures and Checksums | done | [ref-signatures-and-checksums.md](ref-signatures-and-checksums.md) |
| Errors | done | [ref-errors.md](ref-errors.md) |
| Feature Flags | done | [ref-feature-flags.md](ref-feature-flags.md) |
| 1.0 API Surface | done | [1.0-api-surface.md](1.0-api-surface.md) |
| Releases Check Type | done | [releases-check-type.md](releases-check-type.md) |
| Async API | done | [async-api.md](async-api.md) |
| Transport Control | done | [transport-control.md](transport-control.md) |
| Release Scan Pagination | done | [release-scan-pagination.md](release-scan-pagination.md) |
| Custom Backends | done | [custom-backends.md](custom-backends.md) |
| Checksum Verification | done | [checksum-verification.md](checksum-verification.md) |
| Multi-file Install | done | [multi-file-install.md](multi-file-install.md) |
| Custom Asset Matching | done | [custom-asset-matching.md](custom-asset-matching.md) |
| Progress Callback | done | [progress-callback.md](progress-callback.md) |
| S3 Auth Token Removal | done | [s3-auth-token-removal.md](s3-auth-token-removal.md) |
| Post-update Verify | done | [post-update-verify.md](post-update-verify.md) |
| Release Tag URL Encoding | done | [release-tag-url-encoding.md](release-tag-url-encoding.md) |
| Error Network vs HTTP Semantics | done | [error-network-vs-http-semantics.md](error-network-vs-http-semantics.md) |
| Error Variant Granularity | done | [error-variant-granularity.md](error-variant-granularity.md) |
| S3 Max Keys Configurable | done | [s3-max-keys-configurable.md](s3-max-keys-configurable.md) |
| Async Future Extensions | pending | [async-future-extensions.md](async-future-extensions.md) |
| Checksum from Asset | partial | [checksum-from-asset.md](checksum-from-asset.md) |
| Update Config Internal Accessors | done | [update-config-internal-accessors.md](update-config-internal-accessors.md) |
| Releases Test Constructor | done | [releases-test-constructor.md](releases-test-constructor.md) |
| Choose Latest Release Sort | done | [choose-latest-release-sort.md](choose-latest-release-sort.md) |
| Embedded Key Verification | done | [embedded-key-verification.md](embedded-key-verification.md) |
| Corporate Network Config | pending | [corporate-network-config.md](corporate-network-config.md) |
| Restart After Update | done | [ref-restart.md](ref-restart.md) |
| Update-check Interval Guard | done | [ref-check-interval.md](ref-check-interval.md) |

## Conventions

- Each normative statement carries a stable ID (e.g. `FEAT-1`, `API-3`). IDs are
  append-only: retire an ID by marking it removed, never reuse the number.
- Specs are document-first: a feature is documented (status `pending`, or
  `research` if it needs design work) before implementation begins. Flip to
  `done` only once implemented and verified.
- Spec files are named `<slug>.md` and linked from the table above.
- `ref-*.md` files document current behavior cited to `file:line`. They are the
  source of truth for evaluating changes and detecting regressions: if code and a
  `ref-*` spec disagree, one of them is a bug. Update the matching `ref-*` spec
  in the same change that alters behavior.

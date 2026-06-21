# s3 auth_token removal (G9)

Status: implemented

## Problem

The shared setter macro gave every backend an `auth_token` setter, including the s3
`UpdateBuilder`. S3 authenticates via `access_key` (AWS SigV4 signing) and never
consulted `auth_token`, so `.auth_token(..)` on an s3 updater silently did nothing,
a discoverable footgun.

## Decision

The setter macro gained a `no_auth_token` arm that omits the shared setter, and the
s3 backend uses it. In the 1.0 breaking window the s3-specific `auth_token` setter
was removed outright (an interim release had it as a `#[deprecated]` no-op pointing
at `access_key`). The s3 backend authenticates only by signing with `access_key`
(the `s3-auth` feature); `auth_token` remains on github/gitlab/gitea.

See `src/backends/s3.rs`, `impl_common_builder_setters!` in `src/macros.rs`, and the
CHANGELOG `[unreleased]` Removed entry plus the migration guides.

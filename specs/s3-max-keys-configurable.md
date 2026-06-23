# Configurable s3 max-keys

Status: implemented

## Problem

The s3 listing requested a fixed `MAX_KEYS = 100` keys per request with no builder setter to
change it, and a truncated listing was not followed, so a bucket with more than 100 matching
objects was silently truncated.

## Decision

The s3 `UpdateBuilder` and `ReleaseListBuilder` gain a `max_keys(impl Into<u16>)` setter, threaded
into the `max-keys=` query of the listing URL. The const widened from `u8` to a `u16` field on the
builders / `Update` / `ReleaseList` (default 1000, the ListObjectsV2 cap). The setter clamps to
`1..=1000` via `clamp_max_keys`.

The listing now also follows the continuation token: the parser reads `<IsTruncated>true</...>` and
`<NextContinuationToken>`, and when truncated returns `Page::next` as a fresh `PageRequest` with
`continuation-token=<token>` in the query, which the same sans-io driver follows. Under `s3-auth`
each continuation URL is freshly SigV4-signed. So a >1000-key bucket is now walked across multiple
requests, not silently truncated.

A `signature_ttl(Duration)` setter (default 300s) replaces the two hardcoded `300`s, threaded into
the SigV4 signer as the `X-Amz-Expires` of signed listing and download URLs.

See `src/backends/s3.rs` and the CHANGELOG.

# Configurable s3 max-keys

Status: not implemented

## Problem

The s3 listing requests `MAX_KEYS = 100` keys per request (`src/backends/s3.rs`) with
no builder setter to change it. A bucket with more than 100 matching objects is
truncated per page with no way for a caller to widen the page size.

## What it would take

An additive builder setter on the s3 `Update` / `ReleaseList` builders (for example
`max_keys(u16)`) threaded into the request URL where `MAX_KEYS` is currently a
constant. The S3 ListObjects API caps a single request at 1000 keys, so the setter
should clamp or document that bound. Following the existing list paths through all
matching objects would be the more complete fix, but is a larger change.

## Why deferred

An additive setter is possible post-1.0 with no break, so it is not a freeze blocker.
At minimum the per-request cap should be documented in the s3 module docs so the
truncation is not silent.

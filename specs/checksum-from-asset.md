# Checksum from release asset

Status: not implemented

## Problem

Checksum verification requires the caller to pin or fetch the expected digest and pass
it via `checksum(Checksum::Sha256(hex))` (see `checksum-verification.md`). Many
projects publish a `SHA256SUMS` or per-asset `.sha256` file in the same release, so the
digest is already available but the caller has to fetch and parse it themselves.

## What it would take

A setter such as `checksum_from_asset("SHA256SUMS")` that, during the update, downloads
the named sums asset from the same release, parses it (the loosely standardized
`<hex>  <filename>` line format, or a bare digest for a single-asset `.sha256`), and
matches the line for the selected asset to derive the expected digest. It reuses the
existing compare-before-install machinery; only the digest source changes. The parsing
must tolerate the format variations (two spaces vs one, a `*` binary marker, leading
paths).

## Why deferred

This is sugar over the implemented caller-provided checksum, which already covers the
verification need. It adds an extra network request and format-parsing surface, so it
waits until there is demand for the convenience. Tracked as the follow-on to the
caller-pinned digest.

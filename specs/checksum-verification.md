# Checksum verification (G5)

Status: implemented

## Problem

The only integrity check was zipsign signature verification (`signatures` feature).
Many projects instead publish `SHA256SUMS` / `.sha256` files, and callers had no way
to verify a download against a known digest through the crate.

## Decision

A caller-provided expected digest behind a new `checksums` feature:
`Update::configure().checksum(Checksum::Sha256(hex))` (or `Checksum::Sha512(..)`).
The crate hashes the downloaded artifact and compares before installing; a mismatch
aborts with nothing installed. The hash algorithm is selected by the `Checksum`
variant, which is `#[non_exhaustive]` so more algorithms can be added later. The
caller pins or fetches the expected digest, so there is no extra network request and
no sums-file parsing.

Verifying against the digest the forge itself publishes per asset (rather than a
caller-pinned one) is a related, now-implemented path: see `checksum-from-asset.md`.

See the `checksums` feature in `Cargo.toml` and the CHANGELOG `[1.0.0]` Added entry.

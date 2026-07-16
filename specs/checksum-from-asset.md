# Checksum from release asset

Status: partially implemented

## Problem

Checksum verification requires the caller to pin or fetch the expected digest and pass
it via `verify_checksum(Checksum::Sha256(hex))` (see `checksum-verification.md`). The
digest is often already published alongside the release, so the caller having to fetch
and parse it is friction.

## Implemented: forge-published per-asset digest

Github publishes a `sha256:<hex>` content digest per release asset. The github backend
reads it into `ReleaseAsset::digest()` (`None` on gitlab/gitea/s3, whose APIs publish
none; a custom `ReleaseSource` attaches one via `ReleaseAsset::with_digest`). Under the
`checksums` feature the update pipeline verifies the download against that digest by
default, gated by the `verify_release_digest(bool)` builder setter. The forge form is
parsed by `Checksum::parse_digest("algorithm:hex")` (`sha256`/`sha512`); an unsupported
or malformed digest is a hard error, not a silent skip. See
`ref-signatures-and-checksums.md` for the full behavior and the CHANGELOG `[unreleased]`
Added entry.

The digest is an integrity check only (the forge recomputes it when an asset is
replaced), so it is not a substitute for the `signatures` feature.

## Deferred: SHA256SUMS-file fetch and parse

Still not implemented: a setter such as `checksum_from_asset("SHA256SUMS")` that, during
the update, downloads a named sums asset from the same release, parses the loosely
standardized `<hex>  <filename>` line format (or a bare digest for a single-asset
`.sha256`), and matches the line for the selected asset to derive the expected digest.
This is sugar over the caller-provided checksum and adds an extra network request plus
format-parsing surface (two spaces vs one, a `*` binary marker, leading paths), so it
waits until there is demand. The forge-published digest above covers the common github
case without any of it.

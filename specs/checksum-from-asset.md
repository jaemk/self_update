# Checksum from release asset

Status: implemented

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

## Implemented: SHA256SUMS-file fetch and parse

`checksum_from_asset("SHA256SUMS")` names an asset of the same release to resolve the
expected digest from. The update fetches it after the artifact is selected and confirmed
and before the artifact itself is downloaded, so a release missing the sums asset fails
without first pulling the whole artifact, then matches the entry for the selected asset's
file name.

The loosely standardized format is handled by `Checksum::from_sums_file`
(`src/checksum.rs`): coreutils text (`<hex>  <name>`) and binary (`<hex> *<name>`) modes,
any whitespace run between the two, leading path components on the listed name (matched
on its last component, `/` and `\` both), the BSD tag form `SHA256 (<name>) = <hex>`,
`#` comments and blank lines, and a whole-file bare digest for the single-artifact
`<name>.sha256` convention. The algorithm comes from the digest length (64 -> sha256,
128 -> sha512), so a `SHA512SUMS` asset needs no separate setting.

Failing to produce a digest is always `Error::ChecksumSourceInvalid { asset, reason }`,
never a silently skipped verification: the caller opted in to sums verification, so
getting none instead is the outcome worth refusing.

This reaches the release layout the per-asset digest above does not: gitlab, gitea, and
s3, whose APIs publish no digest, plus any repo that publishes its own sums file. The
cost is one extra request per update, and only when the setter is used.

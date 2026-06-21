# Signatures and checksums (reference)

Status: implemented

## Scope

Artifact verification of a downloaded release archive before it is installed.
Two independent, opt-in mechanisms:

- Checksum verification (`checksums` feature): the caller pins a content digest
  they already know (e.g. from a published `SHA256SUMS` file) and the download is
  hashed and compared against it.
- Signature verification (`signatures` feature): zipsign / ed25519ph signatures
  embedded in the archive are verified against caller-supplied public keys.

Both run inside the shared `finish_update` tail (`src/update.rs:770`), after the
archive is downloaded to a temp file and before any extraction or install.

## Behavior

### Checksum verification

Gated entirely on the `checksums` feature: `src/checksum.rs:8` (`#![cfg(feature
= "checksums")]`). The feature enables `sha2` (`Cargo.toml:75`,
`checksums = ["dep:sha2"]`).

The pinned digest is carried by the `Checksum` enum (`src/checksum.rs:31`), a
`#[non_exhaustive]` enum with two variants, `Sha256(String)` and `Sha512(String)`
(`src/checksum.rs:33`, `src/checksum.rs:35`). The variant selects the algorithm
(sha2's `Sha256` / `Sha512`, `src/checksum.rs:13`); the contained `String` is the
expected digest, hex encoded.

A caller pins a digest with the builder method `Update::configure().checksum(..)`
(`src/macros.rs:416`), which stores `Some(checksum)` on the common config
(`src/macros.rs:417`). The `UpdateConfig::checksum` accessor returns it
(`src/update.rs:509`, backed by `src/macros.rs:191`).

Verification (`Checksum::verify`, `src/checksum.rs:63`):
- The expected hex string is trimmed (`src/checksum.rs:64`).
- The file is streamed in 8 KiB chunks through the selected digest and lowercase
  hex-encoded (`hash_file`, `src/checksum.rs:80`; `hex_encode`, `src/checksum.rs:94`).
- Comparison is case-insensitive via `eq_ignore_ascii_case`
  (`src/checksum.rs:66`), so upper- or lower-case hex and surrounding whitespace
  are tolerated.
- On mismatch it returns `Error::Update` with a message
  `"<algo> checksum mismatch: expected <expected>, computed <actual>"`
  (`src/checksum.rs:69`). The algorithm tag is `"sha256"` / `"sha512"`
  (`src/checksum.rs:40`).

In the pipeline the checksum gate runs first, only when a checksum was configured
(`src/update.rs:778`-`781`).

### Signature verification

Gated on the `signatures` feature (`Cargo.toml:74`, `signatures =
["dep:zipsign-api"]`). It uses the `zipsign-api` crate. The archive-format
features turn on the matching zipsign verify backends:
`archive-zip = ["zip", "zipsign-api?/verify-zip"]` (`Cargo.toml:69`) and
`archive-tar = ["tar", "zipsign-api?/verify-tar"]` (`Cargo.toml:72`).

A caller supplies ed25519ph public keys with the builder method
`verifying_keys(impl Into<Vec<VerifyingKey>>)` (`src/macros.rs:426`), stored on
the common config (`src/macros.rs:430`). The accessor is
`UpdateConfig::verifying_keys` (`src/update.rs:513`), which defaults to an empty
slice (`src/update.rs:514`, `src/macros.rs:196`).

`verify_signature(archive_path, keys)` (`src/update.rs:905`):
- If no keys are supplied it is a no-op returning `Ok(())`
  (`src/update.rs:909`-`911`). Verification only happens when the feature is on
  AND at least one key is provided.
- The archive kind is detected from the file extension via `detect_archive`
  (`src/update.rs:915`; `detect_archive` at `src/lib.rs:588`).
- The archive's file name is used as the zipsign context; if it is not UTF-8,
  verification fails with `Error::SignatureNonUTF8` (`src/update.rs:918`-`922`).
- The keys are collected with `zipsign_api::verify::collect_keys`
  (`src/update.rs:926`); the archive file is opened (`src/update.rs:928`).
- Dispatch on archive kind (`src/update.rs:930`):
  - `ArchiveKind::Tar(Some(Compression::Gz))` (a `.tar.gz`, under `archive-tar`)
    is verified with `zipsign_api::verify::verify_tar` (`src/update.rs:933`).
  - `ArchiveKind::Zip` (a `.zip`, under `archive-zip`) is verified with
    `zipsign_api::verify::verify_zip` (`src/update.rs:939`).
  - Any other kind (plain, bare `.tar`, etc.) falls through to
    `Err(Error::NoSignatures(archive_kind))` (`src/update.rs:943`,
    `src/update.rs:946`).
- A failed zipsign verification is wrapped into `Error::Signature` via the
  `From<ZipsignError>` impl (`src/errors.rs:168`); the `.map_err(... ::from)`
  calls (`src/update.rs:934`, `src/update.rs:940`) produce a `ZipsignError`.

`detect_archive` only yields `Tar(..)` under `archive-tar` and `Zip` under
`archive-zip` (`src/lib.rs:574`-`579`); without the matching archive feature the
whole `match` block is `#[cfg]`-compiled out (`src/update.rs:916`) and every kind
falls through to `Error::NoSignatures`.

### Ordering within the pipeline

Inside `finish_update` (`src/update.rs:770`), in order:

1. Checksum gate (`#[cfg(feature = "checksums")]`, `src/update.rs:778`-`781`):
   if a checksum is configured, verify it; mismatch returns immediately via `?`.
2. Signature gate (`#[cfg(feature = "signatures")]`, `src/update.rs:783`-`784`):
   `verify_signature` runs; any failure returns via `?`.
3. Archive extraction of the target binary (`src/update.rs:802`).
4. Install via `install_binary` (`src/update.rs:809`), which first runs the
   post-update `verify_with` callback (`src/update.rs:872`-`879`) and only then
   replaces / moves the binary (`src/update.rs:881`-`885`).

So the full verification order is: checksum, then signature, then (after
extraction) the `verify_with` hook, then the binary replacement. The same
`finish_update` tail is shared by both the sync and async flows
(`src/update.rs:861`).

## Public surface

- `self_update::Checksum` enum (`Sha256` / `Sha512`), re-exported under
  `checksums` (`src/lib.rs:492`); `#[non_exhaustive]`.
- `Update::configure().checksum(Checksum)` builder method (`src/macros.rs:416`),
  with `#[doc(alias = "verifying_checksum")]`.
- `self_update::VerifyingKey` type alias = `[u8; zipsign_api::PUBLIC_KEY_LENGTH]`,
  re-exported under `signatures` (`src/lib.rs:465`).
- `self_update::zipsign_api` re-export of the underlying crate, under
  `signatures` (`src/lib.rs:458`).
- `verifying_keys(impl Into<Vec<VerifyingKey>>)` builder method
  (`src/macros.rs:426`).
- Errors: `Error::Update` (checksum mismatch message), `Error::Signature`
  (wrapped `ZipsignError`, `src/errors.rs:65`), `Error::SignatureNonUTF8`
  (`src/errors.rs:69`), `Error::NoSignatures(ArchiveKind)` (`src/errors.rs:58`).

## Invariants and regression checklist

- Verification runs before the binary is committed/replaced: both the checksum and
  signature gates execute before extraction and before `install_binary` replaces
  the executable (`src/update.rs:778`-`813`).
- A checksum mismatch aborts the update: `Checksum::verify` returns an error
  (`src/checksum.rs:69`) propagated by `?` (`src/update.rs:780`), so no
  extraction or install happens.
- A signature-verification failure aborts the update via `?`
  (`src/update.rs:784`).
- Checksum comparison is case-insensitive and trims surrounding whitespace
  (`src/checksum.rs:64`, `src/checksum.rs:66`).
- A SHA-256 hex passed as a `Sha512` (or vice versa) does not match: lengths and
  contents differ.
- Empty `verifying_keys` means signature verification is skipped, not an error
  (`src/update.rs:909`).
- Only `.tar.gz` and `.zip` archives are signature-verifiable; any other kind
  yields `Error::NoSignatures` (`src/update.rs:946`).
- A non-UTF-8 archive file name yields `Error::SignatureNonUTF8`
  (`src/update.rs:922`).
- The signature dispatch arms are `#[cfg]`-gated on the matching archive feature,
  so a kind whose feature is off falls through to `NoSignatures`
  (`src/update.rs:931`, `src/update.rs:937`).

## Tests

- `src/checksum.rs:114` `sha256_matches_known_digest`: known digest matches;
  upper-case and surrounding whitespace are tolerated.
- `src/checksum.rs:127` `sha512_matches_known_digest`: known SHA-512 digest matches.
- `src/checksum.rs:135` `mismatch_is_rejected`: an all-zero digest is rejected, and
  a SHA-256 digest used as a `Sha512` is rejected.
- `src/update.rs:1406` `finish_update_rejects_a_mismatched_checksum_before_extracting`:
  a bad checksum aborts at the gate with a "checksum mismatch" message, before any
  extraction.
- `src/update.rs:1435` `finish_update_passes_a_matching_checksum_then_proceeds`:
  a matching checksum passes the gate, so the failure instead comes later from
  extraction (proving the gate did not abort).
- `src/errors.rs:368` checks the signatures-gated non-UTF8 variant is named
  `SignatureNonUTF8`; `src/errors.rs:329` / `src/errors.rs:346` cover the boxed
  `Signature` error's `Display` and `source()`.

## Related

- `checksum-verification.md`
- `checksum-from-asset.md`
- `post-update-verify.md`
- `ref-update-pipeline.md`

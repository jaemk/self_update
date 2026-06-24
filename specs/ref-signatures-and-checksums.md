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

Both run inside the shared `finish_update` tail (`src/update.rs:798`), after the
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

A caller pins a digest with the builder method
`Update::configure().verify_checksum(..)` (`src/macros.rs:439`), which stores
`Some(checksum)` on the common config (`src/macros.rs:440`). The
`UpdateConfig::verify_checksum` accessor returns it (`src/update.rs:537`, backed by
`src/macros.rs:191`).

Verification (`Checksum::verify`, `src/checksum.rs:55`):
- The expected hex string is trimmed (`src/checksum.rs:56`).
- The file is streamed in 8 KiB chunks through the selected digest and lowercase
  hex-encoded (`hash_file`, `src/checksum.rs:70`; `hex_encode`, `src/checksum.rs:84`).
- Comparison is case-insensitive via `eq_ignore_ascii_case`
  (`src/checksum.rs:58`), so upper- or lower-case hex and surrounding whitespace
  are tolerated.
- On mismatch it returns `Error::ChecksumMismatch { expected, computed }`
  (`src/checksum.rs:61`), whose `Display` is
  `"ChecksumMismatchError: checksum mismatch (expected <e>, computed <c>)"`
  (`src/errors.rs:153`-`158`). Both fields are lowercase hex digests.

In the pipeline the checksum gate runs first, only when a checksum was configured
(`src/update.rs:806`-`809`).

### Signature verification

Gated on the `signatures` feature (`Cargo.toml:74`, `signatures =
["dep:zipsign-api"]`). It uses the `zipsign-api` crate. The archive-format
features turn on the matching zipsign verify backends:
`archive-zip = ["zip", "zipsign-api?/verify-zip"]` (`Cargo.toml:69`) and
`archive-tar = ["tar", "zipsign-api?/verify-tar"]` (`Cargo.toml:72`).

A caller supplies ed25519ph public keys with the builder method
`verify_keys(impl Into<Vec<VerifyingKey>>)` (`src/macros.rs:455`), stored on
the common config (`src/macros.rs:459`). The accessor is
`UpdateConfig::verify_keys` (`src/update.rs:541`), which defaults to an empty
slice (`src/update.rs:542`, `src/macros.rs:196`).

`verify_signature(archive_path, keys)` (`src/update.rs:933`):
- If no keys are supplied it is a no-op returning `Ok(())`
  (`src/update.rs:937`-`939`). Verification only happens when the feature is on
  AND at least one key is provided.
- The archive kind is detected from the file extension via `detect_archive`
  (`src/update.rs:943`; `detect_archive` at `src/lib.rs:624`).
- The archive's file name is used as the zipsign context; if it is not UTF-8,
  verification fails with `Error::SignatureNonUTF8` (`src/update.rs:946`-`950`).
- The keys are collected with `zipsign_api::verify::collect_keys`
  (`src/update.rs:953`); the archive file is opened (`src/update.rs:956`).
- Dispatch on archive kind (`src/update.rs:958`):
  - `ArchiveKind::Tar(Some(Compression::Gz))` (a `.tar.gz`, under `archive-tar`)
    is verified with `zipsign_api::verify::verify_tar` (`src/update.rs:961`).
  - `ArchiveKind::Zip` (a `.zip`, under `archive-zip`) is verified with
    `zipsign_api::verify::verify_zip` (`src/update.rs:967`).
  - Any other kind (plain, bare `.tar`, etc.) falls through to
    `Err(Error::NoSignatures(archive_kind))` (`src/update.rs:974`).
- A failed zipsign verification is wrapped into `Error::Signature` via the
  `From<ZipsignError>` impl (`src/errors.rs:256`); the `.map_err(... ::from)`
  calls (`src/update.rs:962`, `src/update.rs:968`) produce a `ZipsignError`.

`detect_archive` only yields `Tar(..)` under `archive-tar` and `Zip` under
`archive-zip` (`src/lib.rs:587`-`597`); without the matching archive feature the
whole `match` block is `#[cfg]`-compiled out (`src/update.rs:944`) and every kind
falls through to `Error::NoSignatures`.

### Ordering within the pipeline

Inside `finish_update` (`src/update.rs:798`), in order:

1. Checksum gate (`#[cfg(feature = "checksums")]`, `src/update.rs:806`-`809`):
   if a checksum is configured, verify it; mismatch returns immediately via `?`.
2. Signature gate (`#[cfg(feature = "signatures")]`, `src/update.rs:811`-`812`):
   `verify_signature` runs; any failure returns via `?`.
3. Archive extraction of the target binary (`src/update.rs:830`).
4. Install via `install_binary` (`src/update.rs:837`), which first runs the
   post-update `verify_with` callback (`src/update.rs:900`-`907`) and only then
   replaces / moves the binary (`src/update.rs:909`-`912`).

So the full verification order is: checksum, then signature, then (after
extraction) the `verify_with` hook, then the binary replacement. The same
`finish_update` tail is shared by both the sync and async flows
(`src/update.rs:889`).

## Public surface

- `self_update::Checksum` enum (`Sha256` / `Sha512`), re-exported under
  `checksums` (`src/lib.rs:500`); `#[non_exhaustive]`.
- `Checksum::sha256(hex: impl Into<String>) -> Result<Self>`: validating constructor
  for a SHA-256 digest. Accepts a 64-character lowercase or uppercase hex string (no
  surrounding whitespace); returns `Error::Config` if the length or characters are wrong.
- `Checksum::sha512(hex: impl Into<String>) -> Result<Self>`: validating constructor
  for a SHA-512 digest. Accepts a 128-character hex string; same error on invalid input.
- `Update::configure().verify_checksum(Checksum)` builder method
  (`src/macros.rs:439`).
- `self_update::VerifyingKey` type alias = `[u8; zipsign_api::PUBLIC_KEY_LENGTH]`,
  re-exported under `signatures` (`src/lib.rs:470`).
- `self_update::zipsign_api` re-export of the underlying crate, under
  `signatures` (`src/lib.rs:462`).
- `verify_keys(impl Into<Vec<VerifyingKey>>)` builder method
  (`src/macros.rs:455`).
- Errors: `Error::ChecksumMismatch { expected, computed }` (checksum mismatch,
  `src/errors.rs:29`), `Error::Signature` (wrapped `ZipsignError`,
  `src/errors.rs:110`), `Error::SignatureNonUTF8` (`src/errors.rs:114`),
  `Error::NoSignatures(ArchiveKind)` (`src/errors.rs:103`).

## Invariants and regression checklist

- Verification runs before the binary is committed/replaced: both the checksum and
  signature gates execute before extraction and before `install_binary` replaces
  the executable (`src/update.rs:806`-`841`).
- A checksum mismatch aborts the update: `Checksum::verify` returns
  `Error::ChecksumMismatch` (`src/checksum.rs:61`) propagated by `?`
  (`src/update.rs:808`), so no extraction or install happens.
- A signature-verification failure aborts the update via `?`
  (`src/update.rs:812`).
- Checksum comparison is case-insensitive and trims surrounding whitespace
  (`src/checksum.rs:56`, `src/checksum.rs:58`).
- A SHA-256 hex passed as a `Sha512` (or vice versa) does not match: lengths and
  contents differ.
- Empty `verify_keys` means signature verification is skipped, not an error
  (`src/update.rs:937`).
- Only `.tar.gz` and `.zip` archives are signature-verifiable; any other kind
  yields `Error::NoSignatures` (`src/update.rs:974`).
- A non-UTF-8 archive file name yields `Error::SignatureNonUTF8`
  (`src/update.rs:950`).
- The signature dispatch arms are `#[cfg]`-gated on the matching archive feature,
  so a kind whose feature is off falls through to `NoSignatures`
  (`src/update.rs:959`, `src/update.rs:965`).
- `Checksum::sha256` and `Checksum::sha512` are validating constructors: a hex
  string of the wrong length or containing non-hex characters returns `Error::Config`
  before any `Checksum` value is constructed. The `Checksum` enum itself remains
  `#[non_exhaustive]` and can still be constructed directly via its variants for
  callers that have already validated the digest externally.
- Unverified-update warning: when neither a checksum nor any signing keys are
  configured, `finish_update_owned` emits `log::warn!` before proceeding. The
  `ctx_is_unverified` predicate (`src/update.rs`) encapsulates this check and is
  tested independently.

## Tests

- `src/checksum.rs` `sha256_matches_known_digest`: known digest matches;
  upper-case and surrounding whitespace are tolerated.
- `src/checksum.rs` `sha512_matches_known_digest`: known SHA-512 digest matches.
- `src/checksum.rs` `mismatch_is_rejected`: an all-zero digest is rejected, and
  a SHA-256 digest used as a `Sha512` is rejected.
- `src/checksum.rs` `mismatch_yields_checksum_mismatch_variant`: a mismatch
  produces `Error::ChecksumMismatch` carrying the expected and computed digests;
  `mismatch_display_contains_expected_and_computed` checks the `Display` starts with
  `ChecksumMismatchError:` and embeds both digests.
- `src/checksum.rs` `sha256_constructor_accepts_valid_hex`: `Checksum::sha256` returns
  `Ok` for a 64-char lowercase hex string.
- `src/checksum.rs` `sha256_constructor_rejects_wrong_length`: returns `Err` for a
  string that is not 64 hex characters.
- `src/checksum.rs` `sha256_constructor_rejects_non_hex`: returns `Err` for a
  64-character string containing non-hex characters.
- `src/checksum.rs` `sha512_constructor_accepts_valid_hex`, `sha512_constructor_rejects_wrong_length`,
  `sha512_constructor_rejects_non_hex`: same coverage for the 128-character SHA-512 case.
- `src/update.rs` `finish_update_rejects_a_mismatched_checksum_before_extracting`:
  a bad checksum aborts at the gate with a "checksum mismatch" message, before any
  extraction.
- `src/update.rs` `finish_update_passes_a_matching_checksum_then_proceeds`:
  a matching checksum passes the gate, so the failure instead comes later from
  extraction (proving the gate did not abort).
- `src/update.rs` `ctx_is_unverified_true_when_nothing_configured`: predicate returns
  `true` when no checksum and no keys are set.
- `src/update.rs` `ctx_is_unverified_false_when_checksum_configured` (checksums
  feature): predicate returns `false` when a checksum is set.
- `src/errors.rs` checks the signatures-gated non-UTF8 variant is named
  `SignatureNonUTF8`; covers the boxed `Signature` error's `Display` and `source()`.

## Related

- `checksum-verification.md`
- `checksum-from-asset.md`
- `post-update-verify.md`
- `ref-update-pipeline.md`

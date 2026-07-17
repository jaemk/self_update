# Signatures and checksums (reference)

Status: implemented

## Scope

Artifact verification of a downloaded release archive before it is installed.
Two independent, opt-in mechanisms:

- Checksum verification (`checksums` feature): the caller pins a content digest
  they already know (e.g. from a published `SHA256SUMS` file) and the download is
  hashed and compared against it.
- Release-published digest verification (`checksums` feature): the backend-provided
  digest of the selected asset (github's per-asset `sha256:<hex>`) is verified
  automatically, on by default, when the asset carries one.
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

In the pipeline the pinned-checksum gate runs first, only when a checksum was
configured (`src/update.rs:1314`-`1316`).

### Release-published digest verification

Also gated on the `checksums` feature. The selected asset may carry a
backend-published content digest in `algorithm:hex` form (github fills
`ReleaseAsset::digest()` from the release API's per-asset `digest` field;
gitlab/gitea/s3 leave it `None`, since their APIs publish none). A custom
`ReleaseSource` supplies one via `ReleaseAsset::with_digest(..)`.

When `verify_release_digest()` is on (the default; opt out with the builder setter
`verify_release_digest(false)`, `src/macros.rs:713`) and the selected asset carries
a digest, the digest is parsed with `Checksum::parse_digest` (`src/checksum.rs:54`)
and verified against the downloaded archive (`src/update.rs:1320`-`1324`).
`parse_digest` splits on the first `:`, matching `sha256`/`sha512`
(case-insensitive, surrounding whitespace ignored) onto the `Checksum` variant; an
unsupported algorithm or a string with no `:` separator returns
`Error::InvalidResponse` naming the digest, so a present-but-unparseable digest is
a hard error rather than a silent skip. An absent digest skips the gate.

This gate is independent of the pinned-checksum gate: when both apply, both must
pass. The digest is an integrity check only (github recomputes it when an asset is
replaced), so it is not a substitute for signature verification.

### Signature verification

Gated on the `signatures` feature (`Cargo.toml:74`, `signatures =
["dep:zipsign-api"]`). It uses the `zipsign-api` crate. The archive-format
features turn on the matching zipsign verify backends:
`archive-zip = ["zip", "zipsign-api?/verify-zip"]` (`Cargo.toml:69`) and
`archive-tar = ["tar", "zipsign-api?/verify-tar"]` (`Cargo.toml:72`).

A caller supplies ed25519ph public keys with the builder method
`verifying_keys(impl Into<Vec<VerifyingKey>>)` (`src/macros.rs:617`, renamed
from `verify_keys`), stored on the common config's `verifying_keys` field
(`src/macros.rs:621`). The doc-hidden accessor keeps the old name:
`UpdateConfig::verify_keys` (`src/update.rs:710`, `src/macros.rs:260`), which
defaults to an empty slice.

`verify_signature(archive_path, keys)` (`src/update.rs:933`) is public and
re-exported at the crate root under `signatures`
(`self_update::verify_signature`, `src/lib.rs`), so a caller that stages a
download itself (e.g. an installer fetching a companion binary) can run the same
check `update()` runs. It takes `impl AsRef<Path>` and a `&[VerifyingKey]` slice:
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

Inside `finish_update_owned` (`src/update.rs:1305`), in order, under
`#[cfg(feature = "checksums")]` (`src/update.rs:1312`-`1325`):

1. Pinned-checksum gate: if a checksum is configured, verify it; mismatch returns
   immediately via `?` (`src/update.rs:1314`-`1316`).
2. Release-digest gate: if `verify_release_digest` is on and the selected asset
   carries a digest, parse and verify it; a mismatch or unparseable digest returns
   via `?` (`src/update.rs:1320`-`1324`).
3. Signature gate (`#[cfg(feature = "signatures")]`): `verify_signature` runs; any
   failure returns via `?`.
4. Archive extraction of the target binary.
5. Install via `install_binary`, which first runs the post-update `verify_binary`
   callback and only then replaces / moves the binary.

So the full verification order is: pinned checksum, then release digest, then
signature, then (after extraction) the `verify_binary` hook, then the binary
replacement. The same `finish_update_owned` tail is shared by both the sync and
async flows.

## Public surface

- `self_update::Checksum` enum (`Sha256` / `Sha512`), re-exported under
  `checksums` (`src/lib.rs:500`); `#[non_exhaustive]`.
- `Checksum::parse_digest("algorithm:hex")` associated fn (`src/checksum.rs:54`),
  parsing the forge `sha256:<hex>` / `sha512:<hex>` form.
- `Update::configure().verify_checksum(Checksum)` builder method
  (`src/macros.rs:692`).
- `Update::configure().verify_release_digest(bool)` builder method
  (`src/macros.rs:713`), default on. `ReleaseAsset::digest()` getter and
  `ReleaseAsset::with_digest(..)` (`src/update.rs:66`, `src/update.rs:44`) expose
  and populate the `algorithm:hex` digest.
- `self_update::VerifyingKey` type alias = `[u8; zipsign_api::PUBLIC_KEY_LENGTH]`,
  re-exported under `signatures` (`src/lib.rs:470`).
- `self_update::zipsign_api` re-export of the underlying crate, under
  `signatures` (`src/lib.rs:462`).
- `verifying_keys(impl Into<Vec<VerifyingKey>>)` builder method
  (`src/macros.rs:617`); the doc-hidden `verify_keys()` accessor keeps its name.
- `self_update::verify_signature(impl AsRef<Path>, &[VerifyingKey])` free
  function, re-exported under `signatures` (`src/update.rs`, `src/lib.rs`), for
  running the signature check standalone (e.g. from an installer).
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
  (`src/update.rs:1314`-`1316`), so no extraction or install happens.
- The release-digest gate is on by default and only fires when the selected asset
  carries a digest; `verify_release_digest(false)` skips it. A mismatch aborts via
  `Error::ChecksumMismatch`, and a present-but-unparseable digest aborts via
  `Error::InvalidResponse` naming the digest (not a silent skip)
  (`src/update.rs:1320`-`1324`).
- The pinned-checksum and release-digest gates are independent: when both apply,
  both must pass.
- `ReleaseAsset::digest()` is `None` on gitlab/gitea/s3 (their APIs publish no
  per-asset digest); only github fills it. The digest is integrity-only (the forge
  recomputes it if an asset is replaced), not a signature substitute.
- A signature-verification failure aborts the update via `?`
  (`src/update.rs:812`).
- Checksum comparison is case-insensitive and trims surrounding whitespace
  (`src/checksum.rs:56`, `src/checksum.rs:58`).
- A SHA-256 hex passed as a `Sha512` (or vice versa) does not match: lengths and
  contents differ.
- An empty `verifying_keys` set means signature verification is skipped, not an
  error (`src/update.rs:937`).
- Only `.tar.gz` and `.zip` archives are signature-verifiable; any other kind
  yields `Error::NoSignatures` (`src/update.rs:974`).
- A non-UTF-8 archive file name yields `Error::SignatureNonUTF8`
  (`src/update.rs:950`).
- The signature dispatch arms are `#[cfg]`-gated on the matching archive feature,
  so a kind whose feature is off falls through to `NoSignatures`
  (`src/update.rs:959`, `src/update.rs:965`).

## Tests

- `src/checksum.rs:105` `sha256_matches_known_digest`: known digest matches;
  upper-case and surrounding whitespace are tolerated.
- `src/checksum.rs:117` `sha512_matches_known_digest`: known SHA-512 digest matches.
- `src/checksum.rs:125` `mismatch_is_rejected`: an all-zero digest is rejected, and
  a SHA-256 digest used as a `Sha512` is rejected.
- `src/checksum.rs:138` `mismatch_yields_checksum_mismatch_variant`: a mismatch
  produces `Error::ChecksumMismatch` carrying the expected and computed digests;
  `src/checksum.rs:166` `mismatch_display_contains_expected_and_computed` checks the
  `Display` starts with `ChecksumMismatchError:` and embeds both digests.
- `src/update.rs` `finish_update_rejects_a_mismatched_checksum_before_extracting`:
  a bad checksum aborts at the gate with a "checksum mismatch" message, before any
  extraction.
- `src/update.rs` `finish_update_passes_a_matching_checksum_then_proceeds`:
  a matching checksum passes the gate, so the failure instead comes later from
  extraction (proving the gate did not abort).
- `src/update.rs` `finish_update_rejects_a_mismatched_release_digest_by_default`:
  with no pinned checksum, a mismatched asset digest aborts at the gate.
- `src/update.rs` `finish_update_passes_a_matching_release_digest_then_proceeds`:
  a matching asset digest passes the gate.
- `src/update.rs` `finish_update_release_digest_opt_out_skips_the_gate`:
  `verify_release_digest(false)` ignores a mismatched digest.
- `src/update.rs` `finish_update_rejects_an_unsupported_release_digest`:
  a `md5:` digest aborts with `Error::InvalidResponse` naming the digest.
- `src/checksum.rs` `parse_digest_supports_sha256_and_sha512` /
  `parse_digest_rejects_unsupported_or_malformed`: the `algorithm:hex` parser.
- `src/backends/github.rs` `github_dto_parses_sample_payload_through_getters`:
  the API `digest` field maps onto `ReleaseAsset::digest()`; a digest-less asset is
  `None`.
- `src/errors.rs:478` checks the signatures-gated non-UTF8 variant is named
  `SignatureNonUTF8`; `src/errors.rs:455` / `src/errors.rs:438` cover the boxed
  `Signature` error's `Display` and `source()`.

## Related

- `checksum-verification.md`
- `checksum-from-asset.md`
- `post-update-verify.md`
- `ref-update-pipeline.md`

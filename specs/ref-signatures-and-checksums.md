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

Both run inside the shared `finish_update` tail (`src/update.rs:finish_update`), after the
archive is downloaded to a temp file and before any extraction or install.

## Behavior

### Checksum verification

Gated entirely on the `checksums` feature: `src/checksum.rs:Checksum` (`#![cfg(feature
= "checksums")]`). The feature enables `sha2` (`Cargo.toml:75`,
`checksums = ["dep:sha2"]`).

The pinned digest is carried by the `Checksum` enum (`src/checksum.rs:Checksum`), a
`#[non_exhaustive]` enum with two variants, `Sha256(String)` and `Sha512(String)`
(`src/checksum.rs:Sha256`, `src/checksum.rs:Sha512`). The variant selects the algorithm
(sha2's `Sha256` / `Sha512`, `src/checksum.rs:Sha256`); the contained `String` is the
expected digest, hex encoded.

A caller pins a digest with the builder method
`Update::configure().verify_checksum(..)` (`src/macros.rs:verify_checksum`), which stores
`Some(checksum)` on the common config (`src/macros.rs:verify_checksum`). The
`UpdateConfig::verify_checksum` accessor returns it (`src/update.rs:UpdateConfig::verify_checksum`, backed by
`src/macros.rs:verify_checksum`).

Verification (`Checksum::verify`, `src/checksum.rs:Checksum::verify`):
- The expected hex string is trimmed (`src/checksum.rs:Checksum::verify`).
- The file is streamed in 8 KiB chunks through the selected digest and lowercase
  hex-encoded (`hash_file`, `src/checksum.rs:hash_file`; `hex_encode`, `src/checksum.rs:hex_encode`).
- Comparison is case-insensitive via `eq_ignore_ascii_case`
  (`src/checksum.rs:eq_ignore_ascii_case`), so upper- or lower-case hex and surrounding whitespace
  are tolerated.
- On mismatch it returns `Error::ChecksumMismatch { expected, computed }`
  (`src/checksum.rs:Checksum::verify`), whose `Display` is
  `"ChecksumMismatchError: checksum mismatch (expected <e>, computed <c>)"`
  (`src/errors.rs:Error::ChecksumMismatch`). Both fields are lowercase hex digests.

In the pipeline the pinned-checksum gate runs first, only when a checksum was
configured (`src/update.rs:finish_update_owned`).

### Published-sums digest verification

Also gated on the `checksums` feature. `checksum_from_asset(name)`
(`src/macros.rs:checksum_from_asset`) names a release asset carrying digests, e.g. `SHA256SUMS`.
The orchestrators resolve it *before* the artifact download (`src/update.rs:update_extended`,
`src/update.rs:update_extended_async`): `sums_asset_for` (`src/update.rs:sums_asset_for`) finds the
named asset on the same release, `build_sums_download` (`src/update.rs:build_sums_download`) fetches
it over the artifact download's transport with progress reporting cleared, and
`checksum_from_sums_bytes` (`src/update.rs:checksum_from_sums_bytes`) hands the body to
`Checksum::from_sums_file` (`src/checksum.rs:Checksum::from_sums_file`).

`from_sums_file` accepts the formats published in practice: coreutils text (`<hex>  <name>`) and
binary (`<hex> *<name>`) modes, any run of whitespace between the two, a listed name carrying
leading `/` or `\` path components (matched on its last component), the BSD tag form
`SHA256 (<name>) = <hex>`, `#` comments and blank lines, and a whole file consisting of one bare
digest (the `<artifact>.sha256` convention). The algorithm comes from the digest's length, 64 hex
characters for sha256 and 128 for sha512, so a `SHA512SUMS` asset needs no extra configuration.
Names are compared exactly.

Every way the lookup can fail to produce a digest is `Error::ChecksumSourceInvalid { asset, reason }`
rather than a skipped check: no such asset on the release, a non-UTF-8 body, no entry for the
selected asset, or an entry whose digest is not a supported length. The resolved digest is verified
by the same `Checksum::verify` as the pinned gate, immediately after it, and independently of it.

### Release-published digest verification

Also gated on the `checksums` feature. The selected asset may carry a
backend-published content digest in `algorithm:hex` form (github fills
`ReleaseAsset::digest()` from the release API's per-asset `digest` field;
gitlab/gitea/s3 leave it `None`, since their APIs publish none). A custom
`ReleaseSource` supplies one via `ReleaseAsset::with_digest(..)`.

When `verify_release_digest()` is on (the default; opt out with the builder setter
`verify_release_digest(false)`, `src/macros.rs:verify_release_digest`) and the selected asset carries
a digest, the digest is parsed with `Checksum::parse_digest` (`src/checksum.rs:Checksum::parse_digest`)
and verified against the downloaded archive (`src/update.rs:finish_update_owned`).
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
`verifying_keys(impl Into<Vec<VerifyingKey>>)` (`src/macros.rs:verifying_keys`, renamed
from `verify_keys`), stored on the common config's `verifying_keys` field
(`src/macros.rs:verifying_keys`). The doc-hidden accessor keeps the old name:
`UpdateConfig::verify_keys` (`src/update.rs:verifying_keys`, `src/macros.rs:verifying_keys`), which
defaults to an empty slice.

`verify_signature(archive_path, keys)` (`src/update.rs:verify_signature`) is public and
re-exported at the crate root under `signatures`
(`self_update::verify_signature`, `src/lib.rs`), so a caller that stages a
download itself (e.g. an installer fetching a companion binary) can run the same
check `update()` runs. It takes `impl AsRef<Path>` and a `&[VerifyingKey]` slice:
- If no keys are supplied it is a no-op returning `Ok(())`
  (`src/update.rs:verify_signature`). Verification only happens when the feature is on
  AND at least one key is provided.
- The archive kind is detected from the file extension via `detect_archive`
  (`src/update.rs:detect_archive`; `detect_archive` at `src/lib.rs:detect_archive`).
- The archive's file name is used as the zipsign context; if it is not UTF-8,
  verification fails with `Error::SignatureNonUTF8` (`src/update.rs:verify_signature`).
- The keys are collected with `zipsign_api::verify::collect_keys`
  (`src/update.rs:collect_keys`); the archive file is opened (`src/update.rs:verify_signature`).
- Dispatch on archive kind (`src/update.rs:verify_signature`):
  - `ArchiveKind::Tar(Some(Compression::Gz))` (a `.tar.gz`, under `archive-tar`)
    is verified with `zipsign_api::verify::verify_tar` (`src/update.rs:verify_tar`).
  - `ArchiveKind::Zip` (a `.zip`, under `archive-zip`) is verified with
    `zipsign_api::verify::verify_zip` (`src/update.rs:verify_zip`).
  - Any other kind (plain, bare `.tar`, etc.) falls through to
    `Err(Error::NoSignatures(archive_kind))` (`src/update.rs:NoSignatures`).
- A failed zipsign verification is wrapped into `Error::Signature` via the
  `From<ZipsignError>` impl (`src/errors.rs:Error::Signature`); the `.map_err(... ::from)`
  calls (`src/update.rs:verify_tar`, `src/update.rs:verify_zip`) produce a `ZipsignError`.

`detect_archive` only yields `Tar(..)` under `archive-tar` and `Zip` under
`archive-zip` (`src/lib.rs:detect_archive`); without the matching archive feature the
whole `match` block is `#[cfg]`-compiled out (`src/update.rs:verify_signature`) and every kind
falls through to `Error::NoSignatures`.

### Ordering within the pipeline

Inside `finish_update_owned` (`src/update.rs:finish_update_owned`), in order, under
`#[cfg(feature = "checksums")]` (`src/update.rs:finish_update_owned`):

1. Pinned-checksum gate: if a checksum is configured, verify it; mismatch returns
   immediately via `?` (`src/update.rs:finish_update_owned`).
2. Release-digest gate: if `verify_release_digest` is on and the selected asset
   carries a digest, parse and verify it; a mismatch or unparseable digest returns
   via `?` (`src/update.rs:finish_update_owned`).
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
  `checksums` (`src/checksum.rs:Checksum`); `#[non_exhaustive]`.
- `Checksum::parse_digest("algorithm:hex")` associated fn (`src/checksum.rs:Checksum::parse_digest`),
  parsing the forge `sha256:<hex>` / `sha512:<hex>` form.
- `Update::configure().verify_checksum(Checksum)` builder method
  (`src/macros.rs:verify_checksum`).
- `Checksum::from_sums_file(sums, file_name)` associated fn
  (`src/checksum.rs:Checksum::from_sums_file`), resolving a digest from a published sums file.
- `Update::configure().checksum_from_asset(name)` builder method
  (`src/macros.rs:checksum_from_asset`).
- `Update::configure().verify_release_digest(bool)` builder method
  (`src/macros.rs:verify_release_digest`), default on. `ReleaseAsset::digest()` getter and
  `ReleaseAsset::with_digest(..)` (`src/update.rs:ReleaseAsset::digest`, `src/update.rs:ReleaseAsset::with_digest`) expose
  and populate the `algorithm:hex` digest.
- `self_update::VerifyingKey` type alias = `[u8; zipsign_api::PUBLIC_KEY_LENGTH]`,
  re-exported under `signatures` (`src/lib.rs:VerifyingKey`).
- `self_update::zipsign_api` re-export of the underlying crate, under
  `signatures` (`src/lib.rs:VerifyingKey`).
- `verifying_keys(impl Into<Vec<VerifyingKey>>)` builder method
  (`src/macros.rs:verifying_keys`), with a matching `verifying_keys()` accessor (renamed from
  `verify_keys()`).
- `self_update::verify_signature(impl AsRef<Path>, &[VerifyingKey])` free
  function, re-exported under `signatures` (`src/update.rs`, `src/lib.rs`), for
  running the signature check standalone (e.g. from an installer).
- Errors: `Error::ChecksumMismatch { expected, computed }` (checksum mismatch,
  `src/errors.rs:Error::ChecksumMismatch`), `Error::Signature` (wrapped `ZipsignError`,
  `src/errors.rs:Error::Signature`), `Error::SignatureNonUTF8` (`src/errors.rs:Error::SignatureNonUTF8`),
  `Error::NoSignatures(ArchiveKind)` (`src/errors.rs:Error::NoSignatures`).

## Invariants and regression checklist

- Verification runs before the binary is committed/replaced: both the checksum and
  signature gates execute before extraction and before `install_binary` replaces
  the executable (`src/update.rs:finish_update_owned`, `src/update.rs:install_binary`).
- A checksum mismatch aborts the update: `Checksum::verify` returns
  `Error::ChecksumMismatch` (`src/checksum.rs:Checksum::verify`) propagated by `?`
  (`src/update.rs:finish_update_owned`), so no extraction or install happens.
- The release-digest gate is on by default and only fires when the selected asset
  carries a digest; `verify_release_digest(false)` skips it. A mismatch aborts via
  `Error::ChecksumMismatch`, and a present-but-unparseable digest aborts via
  `Error::InvalidResponse` naming the digest (not a silent skip)
  (`src/update.rs:finish_update_owned`).
- The pinned-checksum and release-digest gates are independent: when both apply,
  both must pass.
- `ReleaseAsset::digest()` is `None` on gitlab/gitea/s3 (their APIs publish no
  per-asset digest); only github fills it. The digest is integrity-only (the forge
  recomputes it if an asset is replaced), not a signature substitute.
- A signature-verification failure aborts the update via `?`
  (`src/update.rs:finish_update_owned`).
- Checksum comparison is case-insensitive and trims surrounding whitespace
  (`src/checksum.rs:Checksum::verify`, `src/checksum.rs:eq_ignore_ascii_case`).
- A SHA-256 hex passed as a `Sha512` (or vice versa) does not match: lengths and
  contents differ.
- An empty `verifying_keys` set means signature verification is skipped, not an
  error (`src/update.rs:verify_signature`).
- Only `.tar.gz` and `.zip` archives are signature-verifiable; any other kind
  yields `Error::NoSignatures` (`src/update.rs:NoSignatures`).
- A non-UTF-8 archive file name yields `Error::SignatureNonUTF8`
  (`src/update.rs:verify_signature`).
- The signature dispatch arms are `#[cfg]`-gated on the matching archive feature,
  so a kind whose feature is off falls through to `NoSignatures`
  (`src/update.rs:verify_tar`, `src/update.rs:verify_zip`).

## Tests

- `src/checksum.rs:sha256_matches_known_digest` `sha256_matches_known_digest`: known digest matches;
  upper-case and surrounding whitespace are tolerated.
- `src/checksum.rs:sha512_matches_known_digest` `sha512_matches_known_digest`: known SHA-512 digest matches.
- `src/checksum.rs:mismatch_is_rejected` `mismatch_is_rejected`: an all-zero digest is rejected, and
  a SHA-256 digest used as a `Sha512` is rejected.
- `src/checksum.rs:mismatch_yields_checksum_mismatch_variant` `mismatch_yields_checksum_mismatch_variant`: a mismatch
  produces `Error::ChecksumMismatch` carrying the expected and computed digests;
  `src/checksum.rs:mismatch_display_contains_expected_and_computed` `mismatch_display_contains_expected_and_computed` checks the
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
- `src/errors.rs:signature_non_utf8_variant_is_renamed_and_displays` checks the signatures-gated non-UTF8 variant is named
  `SignatureNonUTF8`; `src/errors.rs:signature_error_display_includes_prefix_and_inner_message` / `src/errors.rs:signature_error_is_opaque_with_source` cover the boxed
  `Signature` error's `Display` and `source()`.

## Related

- `checksum-verification.md`
- `checksum-from-asset.md`
- `post-update-verify.md`
- `ref-update-pipeline.md`

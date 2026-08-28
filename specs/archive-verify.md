# Pre-extraction archive verification hook

Status: implemented

## Problem

The crate's built-in content gates over a download are a digest (`verify_checksum`,
`verify_release_digest`) and a zipsign signature (`verifying_keys`). A project whose authenticity
check is none of those has nowhere to run it. The common case is GitHub build-provenance
attestation, verified by shelling out to `gh attestation verify <archive> --repo owner/repo`;
`cosign verify-blob` and other detached-signature schemes have the same shape.

`verify_binary` cannot host it. That hook runs on the *extracted binary*, which is a different file
with a different digest than the artifact the forge attested, so pointing an attestation check at it
always fails. The only way to run one was to abandon the high-level `update()` flow and drive
`ReleaseList` + `Download` + `Extract` + `self_replace` by hand.

## Decision

A second verification closure, over the archive:
`Update::configure().verify_archive(|archive: &Path| -> self_update::Result<()> ..)`.

- It runs inside `finish_update_owned`, after the checksum, release-digest, and signature gates and
  before anything is extracted, in single-binary and bundle mode alike. Running it last among the
  archive gates means a corrupt download is rejected by the cheap built-in digest check before an
  external process is spawned on it.
- `Err(..)` aborts the update with nothing extracted and nothing installed. The failure is
  `Error::ArchiveVerificationRejected { reason }`: a separate variant from the `verify_binary`
  hook's `VerificationRejected`, because the two hooks see different files and a caller registering
  both has to be able to tell which one refused. `Error::archive_verification_rejected("..")` builds
  it and passes through unwrapped; any other error's message becomes the reason.
- The hook is `Fn(&Path) -> Result<()> + Send + Sync + 'static`, stored as the same
  `VerifyCallback` type as `verify_binary`, so both flow through `FinishCtx` to the async path
  unchanged.

See the `verify_archive` setter in `src/macros.rs`, `run_archive_verify_hook` in `src/update.rs`,
the `ArchiveVerificationRejected` variant in `src/errors.rs`, and the ordering section of
`ref-update-pipeline.md`.

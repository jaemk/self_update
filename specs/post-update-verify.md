# Post-update verification hook (G10)

Status: implemented

## Problem

Nothing checked that a freshly installed binary actually works. A download that
succeeds but produces a broken or incompatible binary replaced the running exe with
no recourse, which could brick the tool until a manual reinstall.

## Decision

A user-supplied verification closure:
`Update::configure().verify_binary(|new_exe: &Path| -> self_update::Result<()> ..)`. It runs on the extracted binary before the final swap; returning `Err(..)` aborts the
update with nothing installed, and the hook error's message is carried as the reason of the
resulting `Error::VerificationRejected { reason: Some(..) }` (a hook IO error propagates the same
way). Verifying before the swap (rather than after, then rolling back) sidesteps the ordering
problem of replacing the running exe via `self_replace`, so no rollback machinery is needed. A
typical use runs `new_exe --version`, checks the output, and returns `Ok(())` / `Err(..)`.

See the `verify_binary` setter in `src/macros.rs`, the `DynVerifyFn` type and `install_binary` in
`src/lib.rs` / `src/update.rs`, and the `VerificationRejected` variant in `src/errors.rs`.

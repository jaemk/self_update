# Post-update verification hook (G10)

Status: implemented

## Problem

Nothing checked that a freshly installed binary actually works. A download that
succeeds but produces a broken or incompatible binary replaced the running exe with
no recourse, which could brick the tool until a manual reinstall.

## Decision

A user-supplied verification closure:
`Update::configure().verify_with(|new_exe: &Path| -> bool ..)`. It runs on the
extracted binary before the final swap; returning `false` (or erroring) aborts the
update with nothing installed. Verifying before the swap (rather than after, then
rolling back) sidesteps the ordering problem of replacing the running exe via
`self_replace`, so no rollback machinery is needed. A typical use runs
`new_exe --version` and checks the output.

See the `verify_with` setter in `src/update.rs` / `src/macros.rs` and the CHANGELOG
`[1.0.0]` Added entry.

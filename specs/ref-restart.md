# Restart after update (reference)

Status: implemented

## Scope

Documents the `self_update::restart` module (`src/restart.rs`): the `restart()` and
`restart_with(args)` helpers that relaunch the running executable after an update has replaced it on
disk. No feature gate, no dependencies beyond `std`.

## Behavior

`update()` replaces the on-disk executable via `self_replace` (`src/update.rs`, `install_binary`),
but the live process keeps running the old image until it exits. The `restart` helpers relaunch the
(now-updated) binary so a long-running process picks up the new code immediately.

Both helpers target the running executable's path and inherit the current environment. They share
a private `restart_command(args)` helper that builds a `std::process::Command` for that path with
the given args (`src/restart.rs`), which is the single point the platform paths and the unit test
agree on.

The path is chosen by `restart_exe` (`src/restart.rs`):

1. The path `install_binary` recorded via `record_replaced_exe` (`src/update.rs`) just after an
   in-place `self_replace`. It is the `current_exe()` value read *before* the replace. Only the first
   record in a process is kept. A `bin_install_path` install to some other file records nothing,
   since the running exe was not replaced.
2. Otherwise `std::env::current_exe()`, passed through `strip_deleted_suffix`. On linux/android,
   `current_exe()` reads `/proc/self/exe`, which appends `" (deleted)"` once the running inode is
   unlinked, so after any rename-over replace it names a path that does not exist (#217). When that
   path is missing and the path without the suffix exists, the stripped path is used. In every
   other case, and on every other platform, the path is returned unchanged.

`/proc/self/exe` itself is never exec'd: it resolves to the old, unlinked image, so the restart
would run the pre-update binary.

- `restart() -> Result<Infallible>` (`src/restart.rs`): re-runs with the current arguments, i.e.
  `std::env::args_os().skip(1)`. Implemented as `restart_with(std::env::args_os().skip(1))`.
- `restart_with<I, S>(args) -> Result<Infallible>` where `I: IntoIterator<Item = S>, S: AsRef<OsStr>`
  (`src/restart.rs`): re-runs with `args` verbatim, replacing (not appending to) the original
  arguments. The bound mirrors `Command::args`, so `&str`, `String`, and `OsString` iterators all
  work.

The return type is `std::convert::Infallible`: on success neither helper returns, so a returned
value can only be `Err`.

### Platform behavior

- Unix (`#[cfg(unix)]`): the process image is replaced in place with
  `std::os::unix::process::CommandExt::exec`. `exec` returns only on failure, which is mapped to
  `Err(Error::Io(..))`; on success the call never returns and the PID is preserved.
- Windows (`#[cfg(windows)]`): there is no `exec`. The updated binary is spawned as a separate
  process (inheriting stdio) with `Command::spawn`, then the current process exits with
  `std::process::exit(0)`. The PID therefore changes; a console application's replacement runs in
  the same console. A spawn failure surfaces as `Err(Error::Io(..))` via the `?` on `spawn()`.

## Public surface

- `self_update::restart::restart() -> Result<std::convert::Infallible>` (`src/restart.rs`).
- `self_update::restart::restart_with<I, S>(args: I) -> Result<std::convert::Infallible>`
  where `I: IntoIterator<Item = S>, S: AsRef<std::ffi::OsStr>` (`src/restart.rs`).

## Invariants and regression checklist

- Both helpers re-exec the recorded pre-replace path, else `current_exe()` (suffix-stripped on
  linux/android), never a hard-coded path and never `/proc/self/exe`.
- After an in-place update on linux, the restart target is a real file, not the
  `"<path> (deleted)"` form `current_exe()` reports.
- `restart_with` forwards its args verbatim, in order, with nothing appended and the original
  arguments discarded.
- `restart()` forwards `args_os().skip(1)` (the current arguments minus the program name).
- Success never returns (unix `exec` / windows `exit(0)`); only failures yield `Err(Error::Io)`.
- No feature gate and no new dependencies: the module compiles on a default build.

## Tests

- `src/restart.rs` (`tests` module): `restart_command_targets_current_exe_with_given_args` pins that
  the built command targets `current_exe()` and forwards exactly the supplied args;
  `restart_command_with_no_args_forwards_none` pins that an empty arg list is honored verbatim.
  These run on every platform without replacing the process. `restart_exe_prefers_recorded_path`
  pins the recorded-path-first order; the linux-only `deleted_suffix` tests pin when the
  `" (deleted)"` suffix is stripped and when it is kept.
- `tests/restart.rs`: `restart_with_reexecs_after_exe_replaced_on_disk` runs a private copy of the
  test binary that renames a fresh copy over its own path (as `self_replace` does) and then calls
  `restart_with`, asserting the exec succeeds and forwards the args. It covers the suffix fallback
  end to end (#217). The recorded-path wiring in `install_binary` is not driven end to end, because
  that would self-replace the test binary.
- `tests/restart.rs` (`#[cfg(unix)]`): `restart_with_reexecs_current_exe_with_given_args` drives a
  real `exec` end to end. It spawns a fresh copy of the test binary that execs itself via
  `restart_with`, and asserts the re-executed process observed exactly the forwarded arguments. The
  windows spawn-then-exit path is verified manually (a detached spawn followed by `exit(0)` is racy
  to assert on in an automated test).

## Related

- `ref-update-pipeline.md` (the `install_binary` / `self_replace` step this composes with).
- `ref-check-interval.md` (the companion application-flow helper).

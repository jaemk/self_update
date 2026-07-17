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

Both helpers target `std::env::current_exe()` and inherit the current environment. They share a
private `restart_command(args)` helper that builds a `std::process::Command` for the current exe
with the given args (`src/restart.rs`), which is the single point the platform paths and the unit
test agree on.

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

- Both helpers re-exec `current_exe()`, never a hard-coded path.
- `restart_with` forwards its args verbatim, in order, with nothing appended and the original
  arguments discarded.
- `restart()` forwards `args_os().skip(1)` (the current arguments minus the program name).
- Success never returns (unix `exec` / windows `exit(0)`); only failures yield `Err(Error::Io)`.
- No feature gate and no new dependencies: the module compiles on a default build.

## Tests

- `src/restart.rs` (`tests` module): `restart_command_targets_current_exe_with_given_args` pins that
  the built command targets `current_exe()` and forwards exactly the supplied args;
  `restart_command_with_no_args_forwards_none` pins that an empty arg list is honored verbatim.
  These run on every platform without replacing the process.
- `tests/restart.rs` (`#[cfg(unix)]`): `restart_with_reexecs_current_exe_with_given_args` drives a
  real `exec` end to end. It spawns a fresh copy of the test binary that execs itself via
  `restart_with`, and asserts the re-executed process observed exactly the forwarded arguments. The
  windows spawn-then-exit path is verified manually (a detached spawn followed by `exit(0)` is racy
  to assert on in an automated test).

## Related

- `ref-update-pipeline.md` (the `install_binary` / `self_replace` step this composes with).
- `ref-check-interval.md` (the companion application-flow helper).

/*! Restart the current process after an update.

After [`update()`](crate::update::ReleaseUpdate::update) reports
[`VersionStatus::Updated`](crate::VersionStatus::Updated) it has already replaced the running
executable on disk (via [`self_replace`](https://crates.io/crates/self-replace)), but the *live*
process keeps running the old code until it exits. To pick up the new binary immediately, restart
into it.

Two entry points:

- [`restart`] re-runs the new binary with the **same arguments** the current process was launched
  with (`std::env::args_os().skip(1)`).
- [`restart_with`] re-runs it with **arguments you supply**, for the common case of dropping a flag
  so the restarted process does not immediately update again (e.g. a `--upgrade` subcommand that
  should not recurse).

Both inherit the current environment. On success neither returns (the return type is
[`Infallible`](std::convert::Infallible)); they yield `Err` only when the restart itself could not
be started.

## Platform behavior

- **Unix:** the process image is replaced in place with
  [`exec`](std::os::unix::process::CommandExt::exec), so the PID is preserved and nothing returns on
  success.
- **Windows:** there is no `exec`. The new binary is **spawned** as a separate process (inheriting
  stdio) and the current process then exits with code `0`. The PID therefore changes, and a console
  application's replacement runs in the same console.

Because the running executable was already replaced on disk, the restart launches the *updated*
binary, not the old one.

```rust,no_run
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("myapp")
        .current_version(self_update::cargo_crate_version!())
        .build()?
        .update()?;

    if status.is_updated() {
        // Relaunch without the `--upgrade` flag so the new process runs normally
        // instead of trying to update itself again.
        self_update::restart::restart_with(["run"])?;
    }
    Ok(())
}
```
*/

use crate::errors::*;
use std::convert::Infallible;
use std::ffi::OsStr;
use std::process::Command;

/// Build the [`Command`] that restarts the current process: the current executable
/// ([`current_exe`](std::env::current_exe)) with `args` and the inherited environment.
///
/// Factored out so the argument/target wiring is testable without actually replacing the process.
fn restart_command<I, S>(args: I) -> Result<Command>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.args(args);
    Ok(cmd)
}

/// Restart the current process, re-running the (already updated) executable with the **same
/// arguments** it was launched with.
///
/// Equivalent to [`restart_with`] passing `std::env::args_os().skip(1)`. See the
/// [module docs](crate::restart) for platform behavior. On success this does not return; it yields
/// `Err(Error::Io(..))` only if the restart could not be started.
pub fn restart() -> Result<Infallible> {
    restart_with(std::env::args_os().skip(1))
}

/// Restart the current process, re-running the (already updated) executable with the arguments in
/// `args` (replacing, not appending to, the original arguments) and the inherited environment.
///
/// Use this to relaunch with a different argument list than the current process was started with,
/// e.g. dropping an `--upgrade` flag so the restarted process runs normally instead of updating
/// again. See the [module docs](crate::restart) for platform behavior. On success this does not
/// return; it yields `Err(Error::Io(..))` only if the restart could not be started.
///
/// ```rust,no_run
/// # fn run() -> Result<(), Box<dyn std::error::Error>> {
/// // Relaunch the updated binary with a fresh argument list.
/// self_update::restart::restart_with(["run", "--quiet"])?;
/// # Ok(())
/// # }
/// ```
#[cfg(unix)]
pub fn restart_with<I, S>(args: I) -> Result<Infallible>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    use std::os::unix::process::CommandExt;
    let mut cmd = restart_command(args)?;
    // `exec` replaces the current process image; it returns only on failure.
    Err(Error::Io(cmd.exec()))
}

/// See the unix version above; on windows there is no `exec`, so this spawns the updated binary as
/// a new process (inheriting stdio) and then exits the current process with code `0`.
#[cfg(windows)]
pub fn restart_with<I, S>(args: I) -> Result<Infallible>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = restart_command(args)?;
    // No `exec` on windows: spawn the replacement, then exit so only the new process remains.
    cmd.spawn()?;
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::restart_command;
    use std::ffi::OsStr;

    // `restart_command` targets the current executable and forwards exactly the supplied args, in
    // order, with nothing appended. This pins the wiring `restart_with` relies on, on every
    // platform, without actually replacing the process (which `exec`/spawn-then-exit make
    // impossible to observe in-process). The unix end-to-end exec path is covered by the
    // integration test in `tests/restart.rs`.
    #[test]
    fn restart_command_targets_current_exe_with_given_args() {
        let cmd = restart_command(["run", "--quiet"]).unwrap();
        assert_eq!(cmd.get_program(), std::env::current_exe().unwrap());
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(args, ["run", "--quiet"]);
    }

    // An empty argument list is honored verbatim (the restarted process gets no args), not silently
    // backfilled from the current process's args.
    #[test]
    fn restart_command_with_no_args_forwards_none() {
        let cmd = restart_command(std::iter::empty::<&OsStr>()).unwrap();
        assert_eq!(cmd.get_args().count(), 0);
    }
}

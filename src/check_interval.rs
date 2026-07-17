/*! Throttle how often an application checks for updates.

Every call to [`update()`](crate::update::ReleaseUpdate::update) (or `is_update_available()`) makes
a network request. An application that would otherwise check on every run can gate that behind
[`UpdateCheckGuard`], a small timestamp-stamp-file guard: it records the time of the last check in a
file you nominate and reports whether enough time has passed to check again.

This is intentionally a guard, not a scheduler: it does not spawn threads or timers, and it stores
nothing but a single unix-epoch-seconds timestamp (no `chrono`/`time` dependency). It is also not a
preferences store; where the stamp file lives and what interval to use are the application's
decisions.

```rust,no_run
use std::time::Duration;
use self_update::check_interval::UpdateCheckGuard;

fn maybe_update() -> Result<(), Box<dyn std::error::Error>> {
    // The caller owns the path. A real app typically builds it from a per-user cache directory
    // (e.g. the `dirs` crate's `dirs::cache_dir()`) added as the application's own dependency.
    let stamp = std::env::temp_dir().join("myapp/update-check");
    let guard = UpdateCheckGuard::new(stamp, Duration::from_secs(24 * 60 * 60));

    if guard.should_check()? {
        // ... run the self_update check/update here ...
        guard.record_check()?;
    }
    Ok(())
}
```

## Semantics

[`should_check`](UpdateCheckGuard::should_check) returns `true` (a check is due) when:

- the stamp file does not exist yet (first run),
- its contents are not a valid timestamp (a corrupt stamp self-heals: it is treated as due, not as
  an error), or
- the recorded time is at least `interval` in the past.

A stamp dated in the *future* (clock skew, or a stamp copied from another machine) also counts as
due. Only a genuine IO error reading the file (e.g. a permissions failure) surfaces as `Err`; a
missing file does not.

[`record_check`](UpdateCheckGuard::record_check) writes the current time by writing to a temporary
file in the same directory and renaming it over the stamp path, so a concurrent reader never
observes a half-written stamp.
*/

use crate::errors::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A timestamp-stamp-file guard that throttles how often an application checks for updates. See the
/// [module docs](crate::check_interval) for the full model and an example.
#[derive(Clone, Debug)]
pub struct UpdateCheckGuard {
    stamp_path: PathBuf,
    interval: Duration,
}

impl UpdateCheckGuard {
    /// Build a guard that records its last-check timestamp at `stamp_path` and considers a new check
    /// due once `interval` has elapsed since that timestamp. The path and interval are the caller's
    /// choice; the file and its parent directory need not exist yet (they are created by
    /// [`record_check`](Self::record_check)).
    pub fn new(stamp_path: impl Into<PathBuf>, interval: Duration) -> Self {
        Self {
            stamp_path: stamp_path.into(),
            interval,
        }
    }

    /// Whether an update check is due: `true` when the stamp file is missing, holds an unparseable
    /// timestamp, is dated in the future, or is at least `interval` old. See the
    /// [module docs](crate::check_interval) for the exact rules. Returns `Err` only on a genuine IO
    /// error reading the stamp (a missing file is not an error).
    pub fn should_check(&self) -> Result<bool> {
        let contents = match std::fs::read_to_string(&self.stamp_path) {
            Ok(s) => s,
            // First run: no stamp yet, so a check is due.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            // A real IO failure (e.g. permissions) is surfaced, not silently treated as due.
            Err(e) => return Err(Error::Io(e)),
        };

        let stamp_secs = match contents.trim().parse::<u64>() {
            Ok(secs) => secs,
            // A corrupt/unparseable stamp self-heals: treat it as due rather than erroring.
            Err(_) => return Ok(true),
        };

        let now_secs = now_epoch_secs();
        // A stamp dated in the future (clock skew) counts as due.
        if now_secs < stamp_secs {
            return Ok(true);
        }
        Ok(now_secs - stamp_secs >= self.interval.as_secs())
    }

    /// Record "now" as the time of the last check, so [`should_check`](Self::should_check) returns
    /// `false` until `interval` has elapsed. Writes to a temporary file in the stamp's directory and
    /// renames it into place so a concurrent reader never sees a partial write. Returns `Err` on an
    /// IO failure (e.g. the directory does not exist or is not writable).
    pub fn record_check(&self) -> Result<()> {
        let secs = now_epoch_secs();
        // Write-to-temp + rename in the same directory so the replacement is atomic and a reader
        // never observes a half-written stamp. Fall back to the current directory when the path has
        // no parent component (a bare filename).
        let dir = match self.stamp_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        write!(tmp, "{secs}")?;
        tmp.flush()?;
        tmp.persist(&self.stamp_path)
            .map_err(|e| Error::Io(e.error))?;
        Ok(())
    }
}

/// Seconds since the unix epoch, clamped at `0` for the (impossible in practice) pre-1970 clock so
/// the guard never panics on a broken clock.
fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::UpdateCheckGuard;
    use std::time::Duration;

    // A missing stamp file (first run) is due, and reading it is not an error.
    #[test]
    fn missing_stamp_is_due() {
        let dir = tempfile::TempDir::new().unwrap();
        let guard = UpdateCheckGuard::new(dir.path().join("stamp"), Duration::from_secs(3600));
        assert!(guard.should_check().unwrap(), "a missing stamp must be due");
    }

    // record_check() then should_check() within the interval reports not-due, and the stamp holds a
    // parseable epoch-seconds value.
    #[test]
    fn record_then_not_due_within_interval() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("stamp");
        let guard = UpdateCheckGuard::new(&path, Duration::from_secs(3600));
        guard.record_check().unwrap();
        assert!(
            !guard.should_check().unwrap(),
            "a freshly recorded check must not be due within the interval"
        );
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.trim().parse::<u64>().is_ok(),
            "record_check must write parseable epoch seconds, got {written:?}"
        );
    }

    // A stamp older than the interval is due.
    #[test]
    fn back_dated_stamp_is_due() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("stamp");
        // A timestamp far in the past (epoch second 1000 ~ 1970).
        std::fs::write(&path, "1000").unwrap();
        let guard = UpdateCheckGuard::new(&path, Duration::from_secs(3600));
        assert!(
            guard.should_check().unwrap(),
            "a stamp older than the interval must be due"
        );
    }

    // A zero interval makes every recorded check due again immediately.
    #[test]
    fn zero_interval_is_always_due() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("stamp");
        let guard = UpdateCheckGuard::new(&path, Duration::from_secs(0));
        guard.record_check().unwrap();
        assert!(
            guard.should_check().unwrap(),
            "a zero interval must always be due"
        );
    }

    // A corrupt/unparseable stamp self-heals: it is treated as due, not as an error.
    #[test]
    fn garbage_stamp_is_due_not_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("stamp");
        std::fs::write(&path, "not-a-timestamp").unwrap();
        let guard = UpdateCheckGuard::new(&path, Duration::from_secs(3600));
        assert!(
            guard.should_check().unwrap(),
            "a corrupt stamp must be treated as due, not error"
        );
    }

    // A stamp dated in the future (clock skew) counts as due.
    #[test]
    fn future_stamp_is_due() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("stamp");
        // Far-future epoch seconds.
        std::fs::write(&path, "99999999999").unwrap();
        let guard = UpdateCheckGuard::new(&path, Duration::from_secs(3600));
        assert!(
            guard.should_check().unwrap(),
            "a future-dated stamp must be treated as due"
        );
    }

    // record_check() into a non-existent directory surfaces an IO error rather than silently
    // succeeding.
    #[test]
    fn record_into_missing_dir_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("does-not-exist").join("stamp");
        let guard = UpdateCheckGuard::new(path, Duration::from_secs(3600));
        assert!(
            guard.record_check().is_err(),
            "record_check into a missing directory must error"
        );
    }

    // An unwritable stamp directory (0o555) makes record_check() fail. Unix-only: the permission
    // model is what makes this deterministic.
    #[cfg(unix)]
    #[test]
    fn record_into_readonly_dir_errors() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let ro = dir.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        let guard = UpdateCheckGuard::new(ro.join("stamp"), Duration::from_secs(3600));
        let result = guard.record_check();
        // Restore write permission so the TempDir can be cleaned up.
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).ok();
        assert!(
            result.is_err(),
            "record_check into a read-only directory must error"
        );
    }

    // A real IO error reading the stamp (here: the stamp path is a directory, so read_to_string
    // fails with something other than NotFound) surfaces as Err, not as a silent "due".
    #[test]
    fn unreadable_stamp_surfaces_io_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("stamp-as-dir");
        std::fs::create_dir(&path).unwrap();
        let guard = UpdateCheckGuard::new(&path, Duration::from_secs(3600));
        assert!(
            guard.should_check().is_err(),
            "a non-NotFound read error must surface, not be treated as due"
        );
    }
}

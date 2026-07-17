# Update-check interval guard (reference)

Status: implemented

## Scope

Documents the `self_update::check_interval` module (`src/check_interval.rs`): the
`UpdateCheckGuard` type that throttles how often an application checks for updates using a
timestamp stamp file. No feature gate; depends only on `std` and the existing `tempfile`
dependency.

## Behavior

Each `update()` / `is_update_available()` call makes a network request. `UpdateCheckGuard` gates
that behind a stamp file recording the last check time (unix epoch seconds), so an application that
would otherwise check on every run checks at most once per interval. It is a guard, not a scheduler:
no threads or timers, and no `chrono`/`time` dependency (the timestamp is stored via
`std::time::SystemTime`).

The guard owns two fields: `stamp_path: PathBuf` and `interval: Duration`
(`src/check_interval.rs`). The caller chooses both; the file and its parent directory need not exist
until `record_check` runs.

- `UpdateCheckGuard::new(stamp_path: impl Into<PathBuf>, interval: Duration) -> Self`
  (`src/check_interval.rs`): construct a guard.
- `should_check(&self) -> Result<bool>` (`src/check_interval.rs`): whether a check is due. Reads the
  stamp with `std::fs::read_to_string`:
  - Missing file (`ErrorKind::NotFound`): due (`Ok(true)`), the first-run case.
  - Any other read error (e.g. permissions, or the path is a directory): surfaced as
    `Err(Error::Io(..))`, not silently treated as due.
  - Contents not parseable as `u64`: due (`Ok(true)`). A corrupt stamp self-heals rather than
    erroring.
  - Parsed stamp dated in the future relative to now (clock skew): due (`Ok(true)`).
  - Otherwise: due iff `now_secs - stamp_secs >= interval.as_secs()`.
- `record_check(&self) -> Result<()>` (`src/check_interval.rs`): stamp the current time. Writes the
  epoch seconds to a `tempfile::NamedTempFile` in the stamp's parent directory (or `.` when the path
  has no parent), flushes, and `persist`s it over `stamp_path`, so the replacement is atomic and a
  concurrent reader never observes a partial write. An IO failure (missing or unwritable directory)
  surfaces as `Err(Error::Io(..))`.

Now is computed by the private `now_epoch_secs()` (`src/check_interval.rs`) as
`SystemTime::now().duration_since(UNIX_EPOCH)` seconds, clamped to `0` on the impossible pre-1970
clock so the guard never panics.

## Public surface

- `self_update::check_interval::UpdateCheckGuard` (`Clone`, `Debug`).
- `UpdateCheckGuard::new(stamp_path: impl Into<PathBuf>, interval: Duration) -> Self`.
- `UpdateCheckGuard::should_check(&self) -> Result<bool>`.
- `UpdateCheckGuard::record_check(&self) -> Result<()>`.

## Invariants and regression checklist

- A missing stamp is due and is not an error; only a genuine (non-`NotFound`) IO read error surfaces.
- A corrupt/unparseable stamp is treated as due (self-healing), not an error.
- A future-dated stamp is due (clock-skew safety).
- `record_check` writes parseable epoch seconds and replaces the stamp atomically
  (temp-file-then-rename in the same directory).
- The caller owns the path; no xdg/dirs dependency is pulled in. No feature gate; only `std` +
  `tempfile`.

## Tests

`src/check_interval.rs` (`tests` module):

- `missing_stamp_is_due`: a first run (no stamp) is due.
- `record_then_not_due_within_interval`: after `record_check`, `should_check` is `false` within the
  interval, and the stamp holds parseable epoch seconds.
- `back_dated_stamp_is_due`: a stamp older than the interval is due.
- `zero_interval_is_always_due`: a zero interval is due immediately after recording.
- `garbage_stamp_is_due_not_error`: a corrupt stamp is due, not an error.
- `future_stamp_is_due`: a future-dated stamp is due.
- `record_into_missing_dir_errors` / `record_into_readonly_dir_errors` (`#[cfg(unix)]` for the 0o555
  case): `record_check` errors when the directory is missing or unwritable.
- `unreadable_stamp_surfaces_io_error`: a non-`NotFound` read error (stamp path is a directory)
  surfaces as `Err`, not "due".

## Related

- `ref-restart.md` (the companion application-flow helper).
- `ref-update-pipeline.md` (the `update()` path this throttles).

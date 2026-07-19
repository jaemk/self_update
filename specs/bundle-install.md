# Bundle Install (directory bundles, #145 phase A)

Status: research (design for maintainer sign-off; not implemented)

## Problem

The update pipeline extracts exactly one file (`bin_path_in_archive`) and replaces
one binary (`install_binary`, `src/update.rs:1523`). A macOS application is a
directory bundle (`MyApp.app/...`): updating only the exe inside it leaves stale
resources and breaks the bundle's code signature. Issue #145 carries a complete
userland implementation (full unzip + dir-copy + self_replace) and the maintainer
welcomed upstreaming it.

Phase A (this spec): a directory-bundle install mode through the existing
pipeline. Phase B (`.deb`/`.msi`) is a docs-only recipe (decided 2026-07-17, see
Non-goals). Phase C (relaunch) shipped as `restart()` / `restart_with()`
(`ref-restart.md`); a swapped bundle composes with it unchanged.

## Building blocks (current behavior, cited)

- `Extract::extract_into` (`src/lib.rs:1034`): full-tree extraction. Zip entries
  get their archived unix mode applied, masked to `0o777` (`lib.rs:1118-1130`;
  tests `lib.rs:3469`, `lib.rs:2737`). Tar unpacks via `tar::Archive::unpack`
  (`lib.rs:1068`), which preserves modes and symlinks. Zip-slip is rejected via
  `enclosed_name` (`lib.rs:1097`).
- Gap: the zip branch materializes symlink entries as regular files containing
  the link target text (`lib.rs:1105-1117`, no `S_IFLNK` branch). See BNDL-4.
- `MoveAll` (`lib.rs:1400`): all-or-nothing multi-file swap. Stashes each
  displaced destination under `temp`, rolls back applied moves in reverse on the
  first failure, returns the original error; rollback is best-effort (failures
  logged via `log::error!`). Rename-only: sources, destinations, and `temp`
  must share one filesystem.
- `install_binary` (`update.rs:1523`): `verify_binary` hook, then `self_replace`
  when `bin_install_path` is the running exe (`same_file`, canonicalizing,
  `update.rs:1554`), else `Move`.
- Config threading: builder setter -> `CommonConfig.bin_install_path` (default
  `current_exe()`, `src/backends/common.rs:574`) -> `FinishCtx` (owned,
  `update.rs:1306`) -> `finish_update_owned` (shared sync/async tail).
- Staging today: download and single-file extraction happen in a system-temp
  `TempDir`.

## BNDL-1: builder API

BNDL-1-1. `bundle_root_in_archive(path: impl Into<String>) -> &mut Self` is added
to the common builder setters (`src/macros.rs`), available on every backend's
`UpdateBuilder`. It names the bundle root directory inside the archive, relative
to the archive root (e.g. `MyApp.app` or `{{ bin }}-{{ version }}/MyApp.app`).
The `{{ bin }}` / `{{ target }}` / `{{ version }}` templates apply with the same
substitution and `is_safe_asset_name` traversal defense as `bin_path_in_archive`
(`update.rs:1426`).

BNDL-1-2. `bundle_install_path(path: impl AsRef<Path>) -> &mut Self` names the
installed bundle directory to replace (e.g. `/Applications/MyApp.app`).

BNDL-1-3. Setting `bundle_root_in_archive` selects bundle mode. Default
`bundle_install_path` on macOS: the nearest ancestor of
`std::env::current_exe()` whose file name ends in `.app`. Resolution happens in
`build()`; no `.app` ancestor and no explicit path => a config error naming the
exe path (see BNDL-5-1). On non-macOS targets there is no default:
`bundle_install_path` is required in bundle mode
(`Error::MissingField { field: "bundle_install_path" }`).

BNDL-1-4. Mutual exclusion: an explicit `bin_path_in_archive(..)` or
`bin_install_path(..)` call combined with bundle mode is a `build()` error. The
`bin_path_in_archive` value auto-derived from `bin_name` does not count
(distinguished by the existing `bin_path_in_archive_auto` flag,
`macros.rs:545`); in bundle mode the auto-derived value is simply unused.

BNDL-1-5. `bin_name` and `current_version` remain required; asset selection,
verification config, confirm/output flags, and progress are unchanged. Bundle
mode is orthogonal to the backend.

BNDL-1-6. There is no exe-in-archive setter. The running exe is located via
`current_exe()` at swap time; the new tree carries its own copy of the exe. The
pipeline verifies the staged bundle root exists and is a directory, and (when
the running exe is inside the installed bundle) that the staged tree contains a
file at the same relative path, before touching the destination.

BNDL-1-7. Async parity: the bundle fields ride through `FinishCtx` so
`update_extended` and `update_extended_async` share the identical finish tail,
as today.

## BNDL-2: pipeline

BNDL-2-1. Download and archive-level verification are unchanged and shared:
checksum, release digest, and signature all run on the downloaded archive bytes
before extraction (`ref-update-pipeline.md`, verify ordering). The archive
still downloads to a system-temp `TempDir`.

BNDL-2-2. Same-filesystem staging: bundle mode creates two directories with
`tempfile::TempDir::new_in(parent)` where `parent` is
`bundle_install_path.parent()`: a staging dir (extraction target) and a stash
dir (displaced-tree holding area). This guarantees every rename in the swap is
same-filesystem, the constraint `MoveAll` documents. Failure to create them
surfaces as an install-path IO error naming `bundle_install_path` (see BNDL-3).
There is no cross-device case by construction, and phase A has no copy
fallback (open question Q5).

BNDL-2-3. Extraction: `Extract::from_source(archive).extract_into(staging)`.
The staged bundle root is `staging/<substituted bundle_root_in_archive>`.
Missing or not a directory => error, nothing touched.

BNDL-2-4. The `verify_binary` hook, when set, runs against the staged bundle
root path before the swap (open question Q6); `Err` aborts as
`Error::VerificationRejected` with nothing replaced, matching `install_binary`.

BNDL-2-5. Swap (stash-and-rollback, `MoveAll` semantics, one code path on all
platforms):

1. If `current_exe()` is inside `bundle_install_path` (ancestor check using the
   `same_file` canonicalization approach, `update.rs:1554`): rename the running
   exe file out to `stash/exe-aside`. Renaming a running executable is
   permitted on unix and windows; this is the primitive `self_replace` itself
   relies on, applied here so the old tree contains no running image before the
   directory rename.
2. Rename `bundle_install_path` -> `stash/old` (the whole old tree, stashed).
3. Rename the staged bundle root -> `bundle_install_path`.
4. On failure at any step, reverse the applied renames in order (restore
   `stash/old`, restore `exe-aside`) and return the original error. Rollback
   is best-effort and logged on failure, exactly the `MoveAll` contract
   (`lib.rs:1496`).
5. On success the stash `TempDir` is dropped. On unix, unlinking the old
   running image is safe (the inode persists until the process exits). On
   windows the aside old exe stays locked until process exit; its deletion is
   scheduled best-effort (self-replace-crate technique) or left to temp
   cleanup. This residue never affects the installed tree.

BNDL-2-6. After a successful swap the file at the running exe's original path
is the new exe from the new tree; no separate `self_replace` call is needed on
the success path. `self_replace`'s rename-the-running-image mechanism is what
step 1 uses; routing the exe through it keeps one code path across platforms
instead of a unix-only "rename the live directory" shortcut.

BNDL-2-7. Windows caveat (documented, not solved in phase A): step 2 fails if
other files inside the old bundle are memory-mapped (e.g. DLLs the process
loaded from the bundle); rollback then restores the original state and the
error names the path. Phase A's target is macOS `.app`; windows/linux
directory bundles work when nothing but the exe is held open (open question
Q4).

BNDL-2-8. `show_output` messages mirror the single-binary flow ("Extracting
archive...", "Replacing bundle directory... Done"). `ReleaseStatus` /
`VersionStatus` reporting is unchanged.

## BNDL-3: preflight and error context (#112 interaction)

BNDL-3-1. The opt-in preflight (`check_install_path_writable`, #112) probes the
bundle's parent directory in bundle mode, not the bundle itself: the swap needs
create+rename permission in the parent (probe via create/delete of a temp
sibling). A definite failure =>
`Error::InstallPathNotWritable { path: <parent> }` before anything downloads.

BNDL-3-2. Independent of preflight, install-step IO errors in the swap carry
the `bundle_install_path` context (the #112 always-on error-context behavior),
so a mid-swap EACCES names the path rather than a bare os error 13.

## BNDL-4: extraction fidelity (standalone fixes)

BNDL-4-1. Permission bits: already correct, no change needed. Zip modes are
applied masked to `0o777` (`lib.rs:1118-1130`); tar preserves modes via
`unpack`. Recorded here because #145's userland code applies `unix_mode()`
manually; upstream already does.

BNDL-4-2. Zip symlinks (bug, fix independently of bundle mode): a zip entry
whose `unix_mode()` has `S_IFLNK` set must be restored as a symlink on unix
(target = entry contents), not written as a regular file as today
(`lib.rs:1105-1117`). A symlink whose target escapes the extraction root
(absolute, or `..`-resolving outside) is rejected, consistent with the
`enclosed_name` zip-slip defense. On windows, symlink entries are written as
regular files (documented; `.app` is not a windows concern). Without this fix
a zipped `.app`'s `Frameworks/*/Versions/Current` links are corrupted and the
publisher's code signature breaks.

## BNDL-5: errors and guarantees

BNDL-5-1. New error variants (naming open, Q7):
- `Error::NoAppBundle { exe: PathBuf }` ("ConfigError: no `.app` ancestor of
  <exe>; set bundle_install_path explicitly") for failed macOS default
  detection.
- A config-conflict error for BNDL-1-4 (either a new
  `Error::ConflictingConfig { .. }` or reuse of the `MissingField` display
  family; decide with the maintainer).

BNDL-5-2. Rollback guarantee: before step 2 of BNDL-2-5 nothing under
`bundle_install_path` has changed. A failure at step 2 or 3 restores the old
tree (and the exe-aside) via reverse renames. After step 3 succeeds the update
is committed. The guarantees match `MoveAll`: all-or-nothing at rename
granularity, original error surfaced, best-effort logged rollback. The bundle
swap adds on top of `MoveAll`: whole-tree granularity (one rename each way, so
no per-file partial window) and the exe-aside step for running-image safety.

## Non-goals

- `.deb` / `.msi` (phase B): docs-only recipe built on `Download` +
  `std::process::Command` handing off to `dpkg -i` / `msiexec /i`; no
  `install_package` helper (decided 2026-07-17). The pipeline's
  replace/verify semantics do not apply to system installers.
- Code signing / notarization: the crate does not sign, staple, or notarize.
  The publisher must ship the archive with a fully signed (and, for
  Gatekeeper, notarized/stapled) `.app`; the swap preserves exactly what was
  shipped. Docs must note: a bundle modified after signing fails Gatekeeper,
  and a quarantined app running under App Translocation executes from a
  read-only randomized mount, so default `.app` detection finds a path that
  cannot be swapped (documented limitation; detection option in Q9).
- No privilege escalation (consistent with #112): an unwritable
  `/Applications` surfaces as an error; sudo/UAC re-exec is the application's
  choice.
- No merge or partial update of bundle contents: whole-root swap only.
- Windows `.app`-equivalent guarantees when the process holds files inside the
  bundle open beyond the exe (BNDL-2-7).

## Tests

- Fixture archives (tar.gz and zip) containing a nested tree with an
  executable-bit file and a relative symlink; assert `extract_into` fidelity
  (mode masked to `0o777`, symlink restored) - extends the existing
  `extract_into_preserves_zip_unix_mode` / zip-slip tests.
- Swap unit tests on temp dirs: fresh install (no existing bundle), replace,
  and injected-failure rollback (e.g. remove the staged root between stash and
  install, or make the destination parent unwritable) asserting the original
  tree is restored byte-for-byte and the original error surfaces.
- Exe-inside-bundle detection: ancestor/`same_file` logic including symlinked
  paths (mirrors the existing `same_file` tests).
- macOS default detection: pure function over a supplied exe path (no real
  `.app` needed) covering `.app` ancestor found / not found / nested `.app`.
- Preflight: parent-dir probe under a 0555 parent (unix), nothing downloaded.
- CI cannot exercise a real `.app` relaunch. Manual test matrix to document in
  the PR: macOS (x86_64 + aarch64) with zip and tar.gz `.app` archives,
  launched from Finder and from a terminal, quarantined (translocated) vs
  cleared, install under `/Applications` and under `~/Applications`;
  post-swap `codesign --verify` and relaunch via `restart()`; windows and
  linux directory-bundle swap with the exe inside and outside the bundle.

## Open questions (maintainer sign-off needed)

Q1. Naming: `bundle_root_in_archive` / `bundle_install_path` vs alternatives
    (`bundle_path_in_archive`, `bundle_dir`). Spec assumes the former.
Q2. Mutual exclusion (BNDL-1-4): hard `build()` error vs last-setter-wins.
    Spec recommends the hard error.
Q3. macOS default detection: on automatically whenever bundle mode is set
    without an explicit path (spec's position), or opt-in only / always
    explicit everywhere.
Q4. Non-macOS in-bundle swaps: allow with documented caveats (spec's
    position) vs hard error on windows when `current_exe()` is inside the
    bundle.
Q5. Cross-device / staging fallback: none (spec's position: staging in the
    destination parent makes cross-device impossible; a copy fallback would
    forfeit atomicity) vs fall back to copy.
Q6. `verify_binary` hook target in bundle mode: staged bundle root (spec's
    position), staged exe path, or skip the hook in bundle mode.
Q7. Error variant names for BNDL-5-1.
Q8. Land the zip-symlink fix (BNDL-4-2) as a standalone bug-fix PR ahead of
    bundle mode? Spec recommends yes.
Q9. App Translocation: detect (`/AppTranslocation/` path component) and fail
    with a specific error, or document-only. Spec leans detect-and-error.

## Related

- `ref-update-pipeline.md` (finish tail; update in the implementing PR)
- `multi-file-install.md` (`MoveAll`)
- `ref-restart.md` (phase C relaunch)
- `post-update-verify.md` (`verify_binary` hook)

//! End-to-end coverage for `self_update::restart::restart_with` on unix, where the restart replaces
//! the process image with `exec` and therefore cannot be observed in-process. `exec` is unix-only,
//! so the whole file is gated on `cfg(unix)`; the cross-platform argument/target wiring is unit
//! tested in `src/restart.rs`, and the windows spawn-then-exit path is verified manually (a
//! detached spawn followed by `exit(0)` is inherently racy to assert on).
//!
//! ## How the test drives a real `exec` without a dedicated helper binary
//!
//! `restart_with` always re-execs `std::env::current_exe()`, so the "helper" has to *be* this test
//! binary. The flow is a three-generation self-re-exec, coordinated entirely through this file's
//! own tests:
//!
//! 1. `restart_with_reexecs_current_exe_with_given_args` (the assertion) spawns a fresh copy of the
//!    test binary filtered to run only `restart_child_entry`, passing an output-file path in the
//!    `SU_RESTART_OUT` env var. Its presence is what tells the child it is a spawned generation
//!    rather than a normal `cargo test` run.
//! 2. That child runs `restart_child_entry` in the "parent" generation (env set, no marker arg yet)
//!    and calls `restart_with([...])` with a marker argument. `exec` replaces it with a third
//!    generation.
//! 3. The third generation runs `restart_child_entry` again, now sees the marker argument, and
//!    writes its own `argv[1..]` to the output file. The assertion reads it back and checks the
//!    args were forwarded to `current_exe` verbatim.
//!
//! Without `SU_RESTART_OUT` set (a normal `cargo test` run), `restart_child_entry` is a no-op, so
//! it never execs the plain test process.

#![cfg(unix)]

use std::process::Command;

const OUT_ENV: &str = "SU_RESTART_OUT";
// A marker arg that distinguishes the post-exec generation from the parent generation. It is also a
// (non-matching) libtest substring filter, so it does not change which test runs.
const MARKER: &str = "su_restart_grandchild_marker";
// The exact args the parent generation asks `restart_with` to forward, and therefore what the
// post-exec generation must observe as its own `argv[1..]`.
const FORWARDED: [&str; 3] = ["restart_child_entry", MARKER, "--nocapture"];

/// Not a real assertion on its own: this is the code the spawned generations run. It is a no-op
/// during an ordinary `cargo test` (when `SU_RESTART_OUT` is unset), so it never execs the plain
/// test process.
#[test]
fn restart_child_entry() {
    let Ok(out_path) = std::env::var(OUT_ENV) else {
        // Ordinary test run, not a spawned generation: do nothing.
        return;
    };

    let is_post_exec = std::env::args().any(|a| a == MARKER);
    if is_post_exec {
        // Third generation: prove `exec` forwarded exactly the args `restart_with` was given.
        let args: Vec<String> = std::env::args().skip(1).collect();
        std::fs::write(&out_path, args.join("\n")).expect("write forwarded args");
    } else {
        // Second generation: exec into the third with a fresh, marker-carrying arg list.
        let err = self_update::restart::restart_with(FORWARDED)
            .expect_err("restart_with returns only on failure");
        panic!("exec failed instead of replacing the process: {err}");
    }
}

/// Spawn a fresh copy of the test binary that execs itself via `restart_with`, and assert the
/// re-executed process saw exactly the forwarded arguments.
#[test]
fn restart_with_reexecs_current_exe_with_given_args() {
    let dir = tempfile::TempDir::new().unwrap();
    let out_path = dir.path().join("forwarded-args.txt");

    let status = Command::new(std::env::current_exe().unwrap())
        // Run only the child entry, exactly.
        .args(["restart_child_entry", "--exact", "--nocapture"])
        .env(OUT_ENV, &out_path)
        .status()
        .expect("spawn test binary");
    assert!(
        status.success(),
        "spawned restart generation exited unsuccessfully: {status:?}"
    );

    let recorded = std::fs::read_to_string(&out_path)
        .expect("post-exec generation should have written the forwarded args");
    let got: Vec<&str> = recorded.lines().collect();
    assert_eq!(
        got, FORWARDED,
        "restart_with must re-exec current_exe with the given args, verbatim"
    );
}

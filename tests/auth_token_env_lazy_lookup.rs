//! A6: `auth_token_from_env()` must not read the environment at all when the token slot is already
//! filled -- observed through the diagnostics the lookup emits.
//!
//! The in-crate unit tests prove `fill_env_token_if_unset_with` does not invoke its resolver
//! closure, but they say nothing about what the *generated setter* passes it: a regression that went
//! back to evaluating `token_from_env(..)` eagerly as an argument would leave every one of them
//! green. The only externally visible effect of the lookup running is its `log::debug!` record
//! naming the variable it used -- the one diagnostic that answers "which credential am I actually
//! sending?" -- so this file installs a logger, sets a variable, and asserts the record's presence
//! and absence around a real builder.
//!
//! It also pins the A5 half of the same rule: a *blank* explicit token does not count as "already
//! filled", so the lookup still runs (and still logs) for `auth_token("")`.
//!
//! # Why this file holds exactly ONE `#[test]`
//!
//! SAFETY: `std::env::set_var` is sound only while no other thread may read the
//! environment concurrently. What holds here is NOT "this process is single-threaded":
//! libtest keeps its harness thread alive in `recv_timeout` while the body runs on a
//! worker at default concurrency. What holds is that no environment-reading thread
//! exists yet -- this binary contains exactly ONE `#[test]`, and every env write below
//! happens BEFORE the first HTTP client is built. That ordering is load-bearing: a
//! reqwest blocking client spawns a background thread that reads `HTTP_PROXY` /
//! `http_proxy`. So do not add a second `#[test]` here, and do not place a `set_var` /
//! `remove_var` after the first `build()` -- either is a genuine data race, not style.
//! (This binary builds no HTTP client at all: the whole contract is observable at
//! setter-call time.) The single global logger it installs is sound for the same reason.
#![cfg(feature = "github")]

use self_update::backends::github;

/// The value the variable is set to. Deliberately distinctive: no captured log record may ever
/// contain it (the pickup diagnostic names the *variable*, never the credential).
const SECRET: &str = "secret-lazy-lookup-token";

/// Capture every `log` record emitted while a closure runs.
mod capture {
    use std::sync::{Mutex, OnceLock};

    struct CaptureLogger;
    static LOGGER: CaptureLogger = CaptureLogger;

    fn buffer() -> &'static Mutex<Vec<String>> {
        static BUF: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
        BUF.get_or_init(|| Mutex::new(Vec::new()))
    }

    impl log::Log for CaptureLogger {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            buffer().lock().unwrap().push(record.args().to_string());
        }
        fn flush(&self) {}
    }

    /// Emitted at the start of every capture. Its absence means the global logger is not ours, in
    /// which case the "nothing was logged" assertion -- the actual A6 assertion -- would pass for
    /// the wrong reason.
    const SENTINEL: &str = "auth-token-env-lazy-lookup-sentinel";

    /// Run `f` and return every log record it emitted (the sentinel excluded).
    pub fn records(f: impl FnOnce()) -> Vec<String> {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            log::set_logger(&LOGGER).expect("this test binary owns the global logger");
            // Trace, so the pickup diagnostic (a `debug!`) is actually captured.
            log::set_max_level(log::LevelFilter::Trace);
        });
        buffer().lock().unwrap().clear();
        log::warn!("{SENTINEL}");
        f();
        let out = buffer().lock().unwrap().clone();
        assert!(
            out.iter().any(|r| r.contains(SENTINEL)),
            "log capture is not active, so a 'did not log' assertion would pass vacuously"
        );
        out.into_iter()
            .filter(|r| !r.contains(SENTINEL))
            .collect::<Vec<_>>()
    }
}

/// The fragment of the pickup diagnostic that identifies it. Emitted by `first_env_token`, i.e.
/// only once the environment has actually been read.
const PICKUP: &str = "using the auth token from";

/// Whether the lookup ran, and (invariant) that no record ever carries the credential itself.
fn looked_up(records: &[String]) -> bool {
    for record in records {
        assert!(
            !record.contains(SECRET),
            "a log record leaked the auth token: {record}"
        );
    }
    records.iter().any(|r| r.contains(PICKUP))
}

/// One test, one process: the environment is read exactly when its result can be used, and not
/// otherwise.
#[test]
fn the_environment_is_only_read_when_the_token_slot_is_blank() {
    // Set before any other thread exists in this process (see the module comment for why).
    unsafe {
        std::env::set_var("GH_TOKEN", SECRET);
    }

    // Positive control first: with an empty slot the lookup runs, finds `GH_TOKEN`, and says so
    // naming the variable. Without this, the negative assertions below would pass just as well on a
    // build where the diagnostic was deleted outright.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0");
    let records = capture::records(|| {
        upd.auth_token_from_env();
    });
    assert!(
        looked_up(&records),
        "an empty slot must actually consult the environment and log which variable it used, got: \
         {records:?}"
    );
    assert!(
        records.iter().any(|r| r.contains("$GH_TOKEN")),
        "the diagnostic must name the variable the token came from, got: {records:?}"
    );
    assert!(upd.has_auth_token());

    // A6 proper: with an explicit token already in the slot there is nothing to fall back to, so the
    // environment must not be read -- and therefore must not claim, in the one log line an operator
    // consults to answer "which credential am I sending?", that the token came from `$GH_TOKEN` when
    // the value actually being sent is the application's own.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token("explicit-token");
    let records = capture::records(|| {
        upd.auth_token_from_env();
    });
    assert!(
        !looked_up(&records),
        "the environment must not be read when an explicit token already fills the slot, got: \
         {records:?}"
    );
    assert!(
        upd.has_auth_token(),
        "the explicit token is of course still configured"
    );

    // The same on the `ReleaseList` builder, whose `auth_token` setter is hand-written per backend
    // rather than macro-generated -- so "the slot is filled" is established by different code there.
    let mut list = github::ReleaseList::configure();
    list.repo_owner("o")
        .repo_name("r")
        .auth_token("explicit-token");
    let records = capture::records(|| {
        list.auth_token_from_env();
    });
    assert!(
        !looked_up(&records),
        "ReleaseList: the environment must not be read for a filled slot, got: {records:?}"
    );

    // A5 x A6: a BLANK explicit token is not "already filled", so the lookup still runs. This is the
    // boundary between the two rules -- an implementation that short-circuited on `slot.is_some()`
    // (the pre-A5 behavior) would fail here while passing every assertion above.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token("   ");
    let records = capture::records(|| {
        upd.auth_token_from_env();
    });
    assert!(
        looked_up(&records),
        "a blank explicit token must not suppress the lookup, got: {records:?}"
    );
    assert!(
        upd.has_auth_token(),
        "and the resolved env token must now be configured"
    );

    // A second `auth_token_from_env()` has a filled slot by then, so it too must stay silent: the
    // idempotent call must not re-read the environment (nor log a second, redundant pickup line).
    let records = capture::records(|| {
        upd.auth_token_from_env();
    });
    assert!(
        !looked_up(&records),
        "a repeated auth_token_from_env() must not re-read the environment, got: {records:?}"
    );
}

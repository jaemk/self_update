//! `build()` must actually *reach* the non-canonical-host warning for an env-sourced token, on
//! **every** builder of every backend.
//!
//! The in-crate unit tests call `warn_if_env_token_off_canonical_host` directly and assert its
//! return value, which says nothing about whether any `build()` calls it: a backend that forgot the
//! call (or passed `false`, or the wrong canonical host) would pass every one of them. The only
//! externally visible effect of the guard is the `log::warn!` record, so this file installs a
//! logger and drives the real builders.
//!
//! It doubles as the observation point for the env-sourced *flag* lifecycle. The flag is
//! crate-internal, and `build()` is the only place that consumes it, so "was the token still
//! env-sourced at build() time?" is exactly "did build() warn?" -- which is what pins
//! `auth_token_from_env()` twice (still env-sourced) and `auth_token(..)` in either order (no
//! longer env-sourced).
//!
//! # Why this file holds exactly ONE `#[test]`
//!
//! `std::env::set_var` is `unsafe` since the 2024 edition: the environment is process-global, and
//! mutating it while another thread reads it (directly, or through libc calls such as
//! `getaddrinfo`) is undefined behavior. Rust's test harness runs the tests of one binary
//! *concurrently on many threads*. Each integration-test file is its own binary and its own
//! process, so with exactly one test here the `set_var` calls happen on the only thread that
//! exists. The single global logger this file installs is sound for the same reason. **Do not add
//! a second `#[test]` to this file**: it would both race the environment and interleave its log
//! records with this one's.
#![cfg(any(
    feature = "github",
    feature = "gitlab",
    feature = "gitea",
    feature = "gitee"
))]

/// The value every variable is set to. Deliberately distinctive: no captured log record may ever
/// contain it (a warning that helpfully printed the token would be a credential leak into logs).
const SECRET: &str = "secret-env-token-value";

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
    /// which case a "nothing was logged" assertion would pass for the wrong reason.
    const SENTINEL: &str = "auth-token-env-host-warning-sentinel";

    /// Run `f` and return every log record it emitted (the sentinel excluded).
    pub fn records(f: impl FnOnce()) -> Vec<String> {
        static INIT: OnceLock<()> = OnceLock::new();
        INIT.get_or_init(|| {
            log::set_logger(&LOGGER).expect("this test binary owns the global logger");
            log::set_max_level(log::LevelFilter::Trace);
        });
        buffer().lock().unwrap().clear();
        log::warn!("{SENTINEL}");
        f();
        let out = buffer().lock().unwrap().clone();
        assert!(
            out.iter().any(|r| r.contains(SENTINEL)),
            "log capture is not active, so a 'did not warn' assertion would pass vacuously"
        );
        out.into_iter()
            .filter(|r| !r.contains(SENTINEL))
            .collect::<Vec<_>>()
    }
}

/// The fragment of the guard's message that identifies it, so an unrelated record (e.g. the
/// `debug!` naming the variable the token came from) cannot be mistaken for a warning.
const WARNING: &str = "resolved from the environment";

/// Whether `records` holds the non-canonical-host warning naming both hosts. Also enforces the
/// invariant that no log record ever carries the token value itself.
fn warned_about(records: &[String], host: &str, canonical: &str) -> bool {
    for record in records {
        assert!(
            !record.contains(SECRET),
            "a log record leaked the auth token: {record}"
        );
    }
    records
        .iter()
        .any(|r| r.contains(WARNING) && r.contains(host) && r.contains(canonical))
}

/// Whether `records` holds the guard's warning at all, whatever hosts it names.
fn warned_at_all(records: &[String]) -> bool {
    for record in records {
        assert!(
            !record.contains(SECRET),
            "a log record leaked the auth token: {record}"
        );
    }
    records.iter().any(|r| r.contains(WARNING))
}

/// One test, one process: every `build()` path is driven with a genuinely env-sourced token and the
/// emitted log records are inspected.
#[test]
fn build_warns_when_an_env_sourced_token_is_bound_to_a_non_canonical_host() {
    // Every backend's variable, set before any other thread exists in this process (see the module
    // comment). Setting them all here keeps the whole file to a single `unsafe` block on the main
    // thread; the per-backend blocks below only read them.
    unsafe {
        std::env::set_var("GH_TOKEN", SECRET);
        std::env::set_var("GITLAB_TOKEN", SECRET);
        std::env::set_var("GITEA_TOKEN", SECRET);
        std::env::set_var("GITEE_TOKEN", SECRET);
    }

    #[cfg(feature = "github")]
    {
        use self_update::backends::github;
        const ENTERPRISE: &str = "https://github.enterprise.test/api/v3";

        // UpdateBuilder: an env-sourced token bound to an enterprise host must warn. This is the
        // case the guard exists for -- an app that exposes its update URL as configuration and runs
        // in CI would otherwise hand `GITHUB_TOKEN` to an attacker-chosen host with no signal.
        let records = capture::records(|| {
            github::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .api_base_url(ENTERPRISE)
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            warned_about(&records, "github.enterprise.test", "api.github.com"),
            "github's UpdateBuilder::build() must reach the guard, got: {records:?}"
        );

        // ReleaseListBuilder: same, through its own separate `build()`.
        let records = capture::records(|| {
            github::ReleaseList::configure()
                .repo_owner("o")
                .repo_name("r")
                .api_base_url(ENTERPRISE)
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            warned_about(&records, "github.enterprise.test", "api.github.com"),
            "github's ReleaseListBuilder::build() must reach the guard, got: {records:?}"
        );

        // The canonical host is the expected case and must stay silent, or every ordinary user gets
        // nagged for using the feature as intended.
        let records = capture::records(|| {
            github::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            !warned_at_all(&records),
            "the default api.github.com must not warn, got: {records:?}"
        );

        // Flag lifecycle, observed at build(). A second `auth_token_from_env()` is a no-op on the
        // token (the slot is already filled), and must NOT clear the env-sourced flag: an
        // implementation assigning `flag = filled` instead of `if filled { flag = true }` would
        // silently drop the warning for an idempotent call.
        let records = capture::records(|| {
            github::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .api_base_url(ENTERPRISE)
                .auth_token_from_env()
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            warned_about(&records, "github.enterprise.test", "api.github.com"),
            "calling auth_token_from_env() twice must leave the token env-sourced, got: {records:?}"
        );

        // ...and an explicit token is the application's own decision about which host to trust, so
        // it must clear the flag in EITHER call order: an enterprise user who passes their own
        // token must not be warned, even with `GH_TOKEN` set in the environment.
        for order in ["explicit then env", "env then explicit"] {
            let records = capture::records(|| {
                let mut b = github::Update::configure();
                b.repo_owner("o")
                    .repo_name("r")
                    .bin_name("app")
                    .current_version("0.1.0")
                    .api_base_url(ENTERPRISE);
                if order == "explicit then env" {
                    b.auth_token("explicit").auth_token_from_env();
                } else {
                    b.auth_token_from_env().auth_token("explicit");
                }
                b.build().unwrap();
            });
            assert!(
                !warned_at_all(&records),
                "an explicit token must clear the env-sourced flag ({order}), got: {records:?}"
            );
        }

        // The same clearing rule on the ReleaseList builder, whose `auth_token` setter is written
        // per backend rather than generated.
        for order in ["explicit then env", "env then explicit"] {
            let records = capture::records(|| {
                let mut b = github::ReleaseList::configure();
                b.repo_owner("o").repo_name("r").api_base_url(ENTERPRISE);
                if order == "explicit then env" {
                    b.auth_token("explicit").auth_token_from_env();
                } else {
                    b.auth_token_from_env().auth_token("explicit");
                }
                b.build().unwrap();
            });
            assert!(
                !warned_at_all(&records),
                "ReleaseList: an explicit token must clear the flag ({order}), got: {records:?}"
            );
        }
    }

    #[cfg(feature = "gitlab")]
    {
        use self_update::backends::gitlab;
        const SELF_HOSTED: &str = "https://gitlab.enterprise.test";

        let records = capture::records(|| {
            gitlab::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .host(SELF_HOSTED)
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            warned_about(&records, "gitlab.enterprise.test", "gitlab.com"),
            "gitlab's UpdateBuilder::build() must reach the guard, got: {records:?}"
        );

        let records = capture::records(|| {
            gitlab::ReleaseList::configure()
                .repo_owner("o")
                .repo_name("r")
                .host(SELF_HOSTED)
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            warned_about(&records, "gitlab.enterprise.test", "gitlab.com"),
            "gitlab's ReleaseListBuilder::build() must reach the guard, got: {records:?}"
        );

        // The default host (gitlab.com) is canonical and must stay silent, on both builders.
        let records = capture::records(|| {
            gitlab::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .auth_token_from_env()
                .build()
                .unwrap();
            gitlab::ReleaseList::configure()
                .repo_owner("o")
                .repo_name("r")
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            !warned_at_all(&records),
            "the default gitlab.com must not warn, got: {records:?}"
        );

        // An explicit token clears the flag in either order (the self-hosted case: this is the user
        // who would otherwise be warned on every build).
        for order in ["explicit then env", "env then explicit"] {
            let records = capture::records(|| {
                let mut b = gitlab::ReleaseList::configure();
                b.repo_owner("o").repo_name("r").host(SELF_HOSTED);
                if order == "explicit then env" {
                    b.auth_token("explicit").auth_token_from_env();
                } else {
                    b.auth_token_from_env().auth_token("explicit");
                }
                b.build().unwrap();
            });
            assert!(
                !warned_at_all(&records),
                "gitlab: an explicit token must clear the flag ({order}), got: {records:?}"
            );
        }
    }

    #[cfg(feature = "gitee")]
    {
        use self_update::backends::gitee;
        const MIRROR: &str = "https://gitee.mirror.test";

        let records = capture::records(|| {
            gitee::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .host(MIRROR)
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            warned_about(&records, "gitee.mirror.test", "gitee.com"),
            "gitee's UpdateBuilder::build() must reach the guard, got: {records:?}"
        );

        let records = capture::records(|| {
            gitee::ReleaseList::configure()
                .repo_owner("o")
                .repo_name("r")
                .host(MIRROR)
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            warned_about(&records, "gitee.mirror.test", "gitee.com"),
            "gitee's ReleaseListBuilder::build() must reach the guard, got: {records:?}"
        );

        // The default host (gitee.com) is canonical and must stay silent, on both builders.
        let records = capture::records(|| {
            gitee::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .auth_token_from_env()
                .build()
                .unwrap();
            gitee::ReleaseList::configure()
                .repo_owner("o")
                .repo_name("r")
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            !warned_at_all(&records),
            "the default gitee.com must not warn, got: {records:?}"
        );

        for order in ["explicit then env", "env then explicit"] {
            let records = capture::records(|| {
                let mut b = gitee::ReleaseList::configure();
                b.repo_owner("o").repo_name("r").host(MIRROR);
                if order == "explicit then env" {
                    b.auth_token("explicit").auth_token_from_env();
                } else {
                    b.auth_token_from_env().auth_token("explicit");
                }
                b.build().unwrap();
            });
            assert!(
                !warned_at_all(&records),
                "gitee: an explicit token must clear the flag ({order}), got: {records:?}"
            );
        }
    }

    #[cfg(feature = "gitea")]
    {
        use self_update::backends::gitea;

        // Gitea is always self-hosted, so it has no canonical host and must NEVER warn -- otherwise
        // every gitea user would be warned on every build, which is exactly the noise that would
        // train them to ignore the github/gitlab/gitee warning. Both builders, env-sourced token,
        // an arbitrary instance host.
        let records = capture::records(|| {
            gitea::Update::configure()
                .host("https://gitea.example.test")
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .auth_token_from_env()
                .build()
                .unwrap();
            gitea::ReleaseList::configure()
                .host("https://gitea.example.test")
                .repo_owner("o")
                .repo_name("r")
                .auth_token_from_env()
                .build()
                .unwrap();
        });
        assert!(
            !warned_at_all(&records),
            "gitea has no canonical host and must never warn, got: {records:?}"
        );
    }
}

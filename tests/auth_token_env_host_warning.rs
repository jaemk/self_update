//! `build()` must actually *reach* the non-canonical-host warning for an env-sourced token, on
//! **every** builder of every backend.
//!
//! The in-crate unit tests call `env_token_host_decision` directly and assert its return value,
//! which says nothing about whether any `build()` calls it: a backend that forgot the call (or
//! passed the wrong canonical host) would pass every one of them. The only externally visible
//! effect of the guard is the `log::warn!` record, so this file installs a logger and drives the
//! real builders.
//!
//! It doubles as the observation point for the env-sourced *flag* lifecycle. The flag is
//! crate-internal, and `build()` is the only place that consumes it, so "was the token still
//! env-sourced at build() time?" is exactly "did build() warn?" -- which is what pins
//! `auth_token_from_env()` twice (still env-sourced) and `auth_token(..)` in either order (no
//! longer env-sourced).
//!
//! It also carries the D4 wire-level half of the same contract: a warning is not, by itself,
//! proof that the token was still attached. github/gitlab/gitee warn-and-send (DECIDED, A1), so a
//! future change turning that guard into a hard block would leave the log-only assertions green;
//! the `Authorization` header assertions below close that gap. Gitea inverts the polarity -- an
//! unacknowledged host is WITHHELD, not warned-and-sent -- so its wire assertions pin the opposite
//! outcome: no header at all, unless the host was acknowledged via `allow_auth_host(..)`.
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
//! The single global logger this file installs is sound for the same reason: no other thread
//! reads or writes it either.
#![cfg(any(
    feature = "github",
    feature = "gitlab",
    feature = "gitea",
    feature = "gitee"
))]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use self_update::http_client::{HeaderMap, HttpClient, HttpResponse};

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

/// D4's wire half: a recording transport, so a `build()` can actually be exercised (`fetch()` /
/// `get_latest_release()`) instead of only inspected for its log output.
mod wire {
    use super::{Duration, HeaderMap, HttpClient, HttpResponse};
    use std::sync::{Arc, Mutex};

    /// A canned, empty release listing. The listing/update call may fail to find a release; these
    /// tests only care about the `Authorization` header the transport was handed.
    pub struct CannedResponse;

    impl HttpResponse for CannedResponse {
        fn headers(&self) -> &HeaderMap {
            static EMPTY: std::sync::OnceLock<HeaderMap> = std::sync::OnceLock::new();
            EMPTY.get_or_init(HeaderMap::new)
        }
        fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
            Box::new(std::io::Cursor::new(b"[]".to_vec()))
        }
    }

    /// A transport that records the `Authorization` header of every request it is handed.
    pub struct AuthRecorder(pub Arc<Mutex<Vec<Option<String>>>>);

    impl HttpClient for AuthRecorder {
        fn get(
            &self,
            _url: &str,
            headers: &HeaderMap,
            _timeout: Option<Duration>,
        ) -> self_update::Result<Box<dyn HttpResponse>> {
            self.0.lock().unwrap().push(
                headers
                    .get(self_update::http::header::AUTHORIZATION)
                    .map(|v| v.to_str().expect("a header value is ASCII").to_string()),
            );
            Ok(Box::new(CannedResponse))
        }
    }
}

/// Run `f` (which must make exactly one request through the `Arc<dyn HttpClient>` it is handed)
/// inside the log capture, returning both the log records it emitted and the `Authorization`
/// header of that single request. Combining the two means the request asserted "still sent" (or
/// "withheld") on the wire is the SAME request whose log output is asserted "warned" (or not).
fn captured_with_header(f: impl FnOnce(Arc<dyn HttpClient>)) -> (Vec<String>, Option<String>) {
    let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_for_closure = seen.clone();
    let records = capture::records(move || {
        f(Arc::new(wire::AuthRecorder(seen_for_closure)));
    });
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.len(),
        1,
        "exactly one request must have gone through the transport, got {seen:?}"
    );
    (records, seen[0].clone())
}

/// The fragment of the guard's message that identifies it, so an unrelated record (e.g. the
/// `debug!` naming the variable the token came from) cannot be mistaken for a warning.
const WARNING: &str = "resolved from the environment";

/// Whether `records` holds the non-canonical-host warn-and-send warning naming both hosts. Also
/// enforces the invariant that no log record ever carries the token value itself. Only meaningful
/// for a backend WITH a canonical host (github/gitlab/gitee) -- gitea's withhold warning has no
/// canonical host to name, see `withheld_about`.
#[cfg(any(feature = "github", feature = "gitlab", feature = "gitee"))]
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

/// Whether `records` holds gitea's withhold warning naming `host`. Distinct from `warned_about`:
/// gitea has no canonical host to compare against, so its warning names only the offending host,
/// not a pair.
#[cfg(feature = "gitea")]
fn withheld_about(records: &[String], host: &str) -> bool {
    for record in records {
        assert!(
            !record.contains(SECRET),
            "a log record leaked the auth token: {record}"
        );
    }
    records
        .iter()
        .any(|r| r.contains(WARNING) && r.contains("withholding") && r.contains(host))
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
/// emitted log records (and, for D4, the resulting wire traffic) are inspected.
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

        // UpdateBuilder: an env-sourced token bound to an enterprise host must warn AND (D4) the
        // token must still reach the wire -- this is the guard's decided behavior (warn-and-send,
        // not a hard block).
        let (records, header) = captured_with_header(|client| {
            let _ = github::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .api_base_url(ENTERPRISE)
                .auth_token_from_env()
                .http_client(client)
                .build()
                .unwrap()
                .get_latest_release();
        });
        assert!(
            warned_about(&records, "github.enterprise.test", "api.github.com"),
            "github's UpdateBuilder::build() must reach the guard, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("token secret-env-token-value"),
            "D4: warn-and-send means the env token must still reach the wire, not just log a warning"
        );

        // ReleaseListBuilder: same, through its own separate `build()`.
        let (records, header) = captured_with_header(|client| {
            let _ = github::ReleaseList::configure()
                .repo_owner("o")
                .repo_name("r")
                .api_base_url(ENTERPRISE)
                .auth_token_from_env()
                .http_client(client)
                .build()
                .unwrap()
                .fetch();
        });
        assert!(
            warned_about(&records, "github.enterprise.test", "api.github.com"),
            "github's ReleaseListBuilder::build() must reach the guard, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("token secret-env-token-value"),
            "D4: warn-and-send means the env token must still reach the wire, not just log a warning"
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

        // A2, the acknowledgement half, on a backend that HAS a canonical host: an
        // `allow_auth_host(host)` entry is the application's explicit "send the token here", so it
        // silences the off-canonical warning exactly as the canonical host does -- while the token
        // still reaches the wire. This is the deliberate enterprise remedy (the alternative being an
        // explicit `auth_token(..)`), and nothing else exercises it end-to-end: the unit tests call
        // `env_token_host_decision` directly, so a `build()` that passed `&[]` instead of the
        // resolved `auth_hosts` would leave them all green and nag every acknowledged deployment on
        // every build.
        let (records, header) = captured_with_header(|client| {
            let _ = github::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .api_base_url(ENTERPRISE)
                .allow_auth_host("github.enterprise.test")
                .auth_token_from_env()
                .http_client(client)
                .build()
                .unwrap()
                .get_latest_release();
        });
        assert!(
            !warned_at_all(&records),
            "an acknowledged host must not warn, even though it is not canonical, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("token secret-env-token-value"),
            "acknowledging the host must silence the warning WITHOUT withholding the token"
        );

        // ...and the control that keeps the assertion above honest: acknowledging some OTHER host
        // does not acknowledge the configured one. An implementation that treated a non-empty
        // `auth_hosts` list as blanket acknowledgement would pass the block above and silently stop
        // reporting every genuinely off-canonical deployment.
        let (records, header) = captured_with_header(|client| {
            let _ = github::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .api_base_url(ENTERPRISE)
                .allow_auth_host("cdn.enterprise.test")
                .auth_token_from_env()
                .http_client(client)
                .build()
                .unwrap()
                .get_latest_release();
        });
        assert!(
            warned_about(&records, "github.enterprise.test", "api.github.com"),
            "acknowledging a different host must not silence the warning, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("token secret-env-token-value"),
            "and warn-and-send still means sent"
        );

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

        let (records, header) = captured_with_header(|client| {
            let _ = gitlab::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .host(SELF_HOSTED)
                .auth_token_from_env()
                .http_client(client)
                .build()
                .unwrap()
                .get_latest_release();
        });
        assert!(
            warned_about(&records, "gitlab.enterprise.test", "gitlab.com"),
            "gitlab's UpdateBuilder::build() must reach the guard, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("Bearer secret-env-token-value"),
            "D4: warn-and-send means the env token must still reach the wire, not just log a warning"
        );

        let (records, header) = captured_with_header(|client| {
            let _ = gitlab::ReleaseList::configure()
                .repo_owner("o")
                .repo_name("r")
                .host(SELF_HOSTED)
                .auth_token_from_env()
                .http_client(client)
                .build()
                .unwrap()
                .fetch();
        });
        assert!(
            warned_about(&records, "gitlab.enterprise.test", "gitlab.com"),
            "gitlab's ReleaseListBuilder::build() must reach the guard, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("Bearer secret-env-token-value"),
            "D4: warn-and-send means the env token must still reach the wire, not just log a warning"
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

        let (records, header) = captured_with_header(|client| {
            let _ = gitee::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .host(MIRROR)
                .auth_token_from_env()
                .http_client(client)
                .build()
                .unwrap()
                .get_latest_release();
        });
        assert!(
            warned_about(&records, "gitee.mirror.test", "gitee.com"),
            "gitee's UpdateBuilder::build() must reach the guard, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("Bearer secret-env-token-value"),
            "D4: warn-and-send means the env token must still reach the wire, not just log a warning"
        );

        let (records, header) = captured_with_header(|client| {
            let _ = gitee::ReleaseList::configure()
                .repo_owner("o")
                .repo_name("r")
                .host(MIRROR)
                .auth_token_from_env()
                .http_client(client)
                .build()
                .unwrap()
                .fetch();
        });
        assert!(
            warned_about(&records, "gitee.mirror.test", "gitee.com"),
            "gitee's ReleaseListBuilder::build() must reach the guard, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("Bearer secret-env-token-value"),
            "D4: warn-and-send means the env token must still reach the wire, not just log a warning"
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

        // Gitea is always self-hosted, so it has NO canonical host -- the opposite polarity from
        // github/gitlab/gitee above (DECIDED, A1): an env-sourced token bound to a host the
        // application never acknowledged is WITHHELD rather than warned-and-sent, and `build()`
        // still succeeds with the request going out anonymous.
        const UNACKNOWLEDGED: &str = "https://gitea.example.test";

        // `has_auth_token()` is checked BEFORE `build()` on both builders (D5): a positive anchor
        // that `GITEA_TOKEN` really was picked up, so a resolver regression (e.g. gitea stops
        // reading that variable) cannot make the withhold assertions below pass vacuously -- with
        // no anchor, "no Authorization header" would also be exactly what a broken resolver
        // produces.
        let mut upd = gitea::Update::configure();
        upd.host(UNACKNOWLEDGED)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token_from_env();
        assert!(
            upd.has_auth_token(),
            "GITEA_TOKEN must still be picked up before build() decides whether to withhold it"
        );
        let (records, header) = captured_with_header(|client| {
            let _ = upd
                .http_client(client)
                .build()
                .unwrap()
                .get_latest_release();
        });
        assert!(
            withheld_about(&records, "gitea.example.test"),
            "gitea's UpdateBuilder::build() must warn that the token is withheld, got: {records:?}"
        );
        assert_eq!(
            header, None,
            "D4: an unacknowledged host must WITHHOLD the token -- no Authorization header on the wire"
        );

        let mut list = gitea::ReleaseList::configure();
        list.host(UNACKNOWLEDGED)
            .repo_owner("o")
            .repo_name("r")
            .auth_token_from_env();
        assert!(
            list.has_auth_token(),
            "GITEA_TOKEN must still be picked up before build() decides whether to withhold it"
        );
        let (records, header) = captured_with_header(|client| {
            let _ = list.http_client(client).build().unwrap().fetch();
        });
        assert!(
            withheld_about(&records, "gitea.example.test"),
            "gitea's ReleaseListBuilder::build() must warn that the token is withheld, got: {records:?}"
        );
        assert_eq!(
            header, None,
            "D4: an unacknowledged host must WITHHOLD the token -- no Authorization header on the wire"
        );

        // A1's remedy: `allow_auth_host(the_same_host)` re-affirms the host, so the token is SENT
        // (D4's positive gitea branch) and the withhold warning is silenced, on both builders.
        let mut upd = gitea::Update::configure();
        upd.host(UNACKNOWLEDGED)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token_from_env()
            .allow_auth_host("gitea.example.test");
        assert!(upd.has_auth_token());
        let (records, header) = captured_with_header(|client| {
            let _ = upd
                .http_client(client)
                .build()
                .unwrap()
                .get_latest_release();
        });
        assert!(
            !warned_at_all(&records),
            "acknowledging the host via allow_auth_host must silence the warning, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("token secret-env-token-value"),
            "an acknowledged host must SEND the env-sourced token"
        );

        let mut list = gitea::ReleaseList::configure();
        list.host(UNACKNOWLEDGED)
            .repo_owner("o")
            .repo_name("r")
            .auth_token_from_env()
            .allow_auth_host("gitea.example.test");
        assert!(list.has_auth_token());
        let (records, header) = captured_with_header(|client| {
            let _ = list.http_client(client).build().unwrap().fetch();
        });
        assert!(
            !warned_at_all(&records),
            "acknowledging the host via allow_auth_host must silence the warning, got: {records:?}"
        );
        assert_eq!(
            header.as_deref(),
            Some("token secret-env-token-value"),
            "an acknowledged host must SEND the env-sourced token"
        );
    }
}

//! End-to-end check of `auth_token_from_env()` on the **github** builders against a real process
//! environment, observed on the wire.
//!
//! This file pins the whole env-token contract on both github builders: pickup, the `GH_TOKEN` >
//! `GITHUB_TOKEN` precedence against a genuinely populated environment, and that an explicit
//! `auth_token(..)` still wins in either call order. It goes one layer further out than the
//! builders' own accessors -- the `Authorization` header a custom transport actually receives --
//! which also covers the `ReleaseList` builder, whose built form exposes no token accessor.
//!
//! # Why this file holds exactly ONE `#[test]`
//!
//! `std::env::set_var` is `unsafe` since the 2024 edition: the environment is process-global, and
//! mutating it while another thread reads it is undefined behavior. Each integration-test file is
//! its own binary and its own process; see the `// SAFETY:` comment at the `set_var` call below for
//! the exact invariant that makes the call sound here. **Do not add a second `#[test]` to this
//! file**; put it in a new single-test file of its own.
#![cfg(feature = "github")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use self_update::backends::github;
use self_update::http_client::{HeaderMap, HttpClient, HttpResponse};

/// A canned, empty release listing. The listing call may fail to find a release; this test only
/// cares about the `Authorization` header the transport was handed.
struct CannedResponse;

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
struct AuthRecorder(Arc<Mutex<Vec<Option<String>>>>);

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

/// Run `f` with a recording transport and return the `Authorization` header of the single request
/// it made. Asserts exactly one request happened, so a build that never reached the transport
/// cannot make a header assertion pass vacuously.
fn auth_header_of(f: impl FnOnce(Arc<dyn HttpClient>)) -> Option<String> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    f(Arc::new(AuthRecorder(seen.clone())));
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.len(),
        1,
        "exactly one request must have gone through the transport, got {seen:?}"
    );
    seen[0].clone()
}

/// One test, one process: the env-resolved token must reach the wire as `token <value>`, and an
/// explicit `auth_token(..)` must beat a **populated** environment in either call order, on both
/// builders.
///
/// `GITHUB_TOKEN` is also exported, and set to a value that must lose: `GH_TOKEN` has precedence
/// over `GITHUB_TOKEN`, matching the `gh` CLI. Exercising that against a real environment (rather
/// than asserting off the declared const list) matters because GitHub Actions auto-populates
/// `GITHUB_TOKEN`, so a reversed list would silently ignore a deliberately exported `GH_TOKEN`.
///
/// The in-crate github tests run in whatever environment cargo was invoked with, which is normally
/// clean, so their precedence assertions hold vacuously -- nothing was resolved to lose to.
#[test]
fn env_token_reaches_the_wire_and_an_explicit_token_still_wins() {
    // SAFETY: `std::env::set_var` is sound only while no other thread may read the
    // environment concurrently. What holds here is NOT "this process is single-threaded":
    // libtest keeps its harness thread alive in `recv_timeout` while the body runs on a
    // worker at default concurrency. What holds is that no environment-reading thread
    // exists yet -- this binary contains exactly ONE `#[test]`, and every env write below
    // happens BEFORE the first HTTP client is built. That ordering is load-bearing: a
    // reqwest blocking client spawns a background thread that reads `HTTP_PROXY` /
    // `http_proxy`. So do not add a second `#[test]` here, and do not place a `set_var` /
    // `remove_var` after the first `build()` -- either is a genuine data race, not style.
    unsafe {
        std::env::set_var("GH_TOKEN", "env-token");
        std::env::set_var("GITHUB_TOKEN", "github-token-must-lose");
    }

    // 1. Pickup on the ReleaseList builder, rendered with github's `token` scheme.
    let mut list = github::ReleaseList::configure();
    list.repo_owner("o").repo_name("r");
    assert!(
        !list.has_auth_token(),
        "nothing is set before the call: the listing would run anonymously"
    );
    list.auth_token_from_env();
    assert!(list.has_auth_token());
    let header = auth_header_of(|client| {
        let _ = list.http_client(client).build().unwrap().fetch();
    });
    assert_eq!(
        header.as_deref(),
        Some("token env-token"),
        "the env-resolved token must reach the wire with github's `token` scheme"
    );

    // 2. Pickup on the Update builder.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token_from_env();
    let header = auth_header_of(|client| {
        let _ = upd
            .http_client(client)
            .build()
            .unwrap()
            .get_latest_release();
    });
    assert_eq!(header.as_deref(), Some("token env-token"));

    // 3. Precedence on the ReleaseList builder (its `auth_token` setter is hand-written per
    //    backend, not macro-generated), in both call orders.
    for (order, list) in [
        ("env then explicit", {
            let mut b = github::ReleaseList::configure();
            b.repo_owner("o")
                .repo_name("r")
                .auth_token_from_env()
                .auth_token("explicit");
            b
        }),
        ("explicit then env", {
            let mut b = github::ReleaseList::configure();
            b.repo_owner("o")
                .repo_name("r")
                .auth_token("explicit")
                .auth_token_from_env();
            b
        }),
    ] {
        let mut list = list;
        let header = auth_header_of(|client| {
            let _ = list.http_client(client).build().unwrap().fetch();
        });
        assert_eq!(
            header.as_deref(),
            Some("token explicit"),
            "an explicit auth_token(..) must beat the populated environment ({order})"
        );
    }

    // 4. The same on the Update builder, observed on the wire rather than through the accessor.
    for (order, upd) in [
        ("env then explicit", {
            let mut b = github::Update::configure();
            b.repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .auth_token_from_env()
                .auth_token("explicit");
            b
        }),
        ("explicit then env", {
            let mut b = github::Update::configure();
            b.repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .auth_token("explicit")
                .auth_token_from_env();
            b
        }),
    ] {
        let mut upd = upd;
        let header = auth_header_of(|client| {
            let _ = upd
                .http_client(client)
                .build()
                .unwrap()
                .get_latest_release();
        });
        assert_eq!(
            header.as_deref(),
            Some("token explicit"),
            "an explicit auth_token(..) must beat the populated environment ({order})"
        );
    }
}

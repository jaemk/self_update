//! End-to-end check of `auth_token_from_env()` on the **gitea** builders against a real process
//! environment, observed on the wire.
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
#![cfg(feature = "gitea")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use self_update::backends::gitea;
use self_update::http_client::{HeaderMap, HttpClient, HttpResponse};

/// The self-hosted instance under test. Gitea has no canonical host, so every gitea user is on one
/// of these; `https` is required for the token to be attached at all.
const HOST: &str = "https://gitea.example.test";

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

/// One test, one process: pins the whole env-token contract on the two gitea builders against a
/// genuinely populated environment, observed where it actually matters -- the `Authorization`
/// header on the wire.
///
/// The in-crate gitea tests run in whatever environment cargo was invoked with, which is normally
/// clean, so their "an explicit token wins over the environment" assertions hold **vacuously**:
/// nothing was resolved to lose to. This file exports `GITEA_TOKEN` first, so the explicit token
/// has something to beat, in *both* call orders (the regression that motivated the fallback
/// semantics: the pair used to be order-sensitive, so an ambient CI credential could displace the
/// application's own). It also pins gitea's `token` scheme (not `Bearer`) for an env-sourced
/// token: gitea and gitlab sit next to each other in this crate and their schemes differ.
///
/// Gitea has no canonical host of its own, so (DECIDED, A1) an env-sourced token bound to a host
/// the application never re-affirmed is WITHHELD, not sent -- the opposite of every other backend
/// in this crate. `HOST` is therefore acknowledged with `allow_auth_host(HOST)` on the pickup
/// steps below, which is the deliberate choice for THIS file: its job is to pin that a genuinely
/// usable env-sourced token reaches the wire with the right scheme, exactly like the sibling
/// per-backend files do for github/gitlab/gitee. The withhold-unless-acknowledged behavior itself,
/// including the unacknowledged (no-header) branch, is pinned on the wire in
/// `tests/auth_token_env_host_warning.rs` (D4), which is also where the in-crate pure-logic
/// coverage of `env_token_host_decision` lives.
#[test]
fn env_token_reaches_the_wire_and_an_explicit_token_still_wins() {
    // Set before any other thread exists in this process (see the module comment for why).
    unsafe {
        std::env::set_var("GITEA_TOKEN", "env-token");
    }

    // 1. Pickup on the ReleaseList builder: the env token is resolved, threaded through build(),
    //    and rendered with gitea's `token` scheme. `allow_auth_host(HOST)` re-affirms the host
    //    (A1's remedy #2), which is what lets this env-sourced token actually reach the wire.
    let mut list = gitea::ReleaseList::configure();
    list.host(HOST)
        .repo_owner("o")
        .repo_name("r")
        .allow_auth_host("gitea.example.test");
    assert!(
        !list.has_auth_token(),
        "nothing is set before the call: the listing would run anonymously"
    );
    list.auth_token_from_env();
    assert!(
        list.has_auth_token(),
        "auth_token_from_env() must pick up GITEA_TOKEN from the process environment"
    );
    let header = auth_header_of(|client| {
        let _ = list.http_client(client).build().unwrap().fetch();
    });
    assert_eq!(
        header.as_deref(),
        Some("token env-token"),
        "the env-resolved token must reach the wire with gitea's `token` scheme, once the host is \
         acknowledged"
    );

    // 2. Pickup on the Update builder, same acknowledgement.
    let mut upd = gitea::Update::configure();
    upd.host(HOST)
        .repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .allow_auth_host("gitea.example.test");
    assert!(!upd.has_auth_token());
    upd.auth_token_from_env();
    assert!(upd.has_auth_token());
    let header = auth_header_of(|client| {
        let _ = upd
            .http_client(client)
            .build()
            .unwrap()
            .get_latest_release();
    });
    assert_eq!(header.as_deref(), Some("token env-token"));

    // 3. Precedence on the Update builder, both call orders. `env -> explicit` is the easy
    //    direction; `explicit -> env` is the one that used to lose.
    for (order, upd) in [
        ("env then explicit", {
            let mut b = gitea::Update::configure();
            b.host(HOST)
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version("0.1.0")
                .auth_token_from_env()
                .auth_token("explicit");
            b
        }),
        ("explicit then env", {
            let mut b = gitea::Update::configure();
            b.host(HOST)
                .repo_owner("o")
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

    // 4. The same precedence on the ReleaseList builder, which carries its own hand-written
    //    `auth_token` setter rather than the macro-generated one.
    for (order, list) in [
        ("env then explicit", {
            let mut b = gitea::ReleaseList::configure();
            b.host(HOST)
                .repo_owner("o")
                .repo_name("r")
                .auth_token_from_env()
                .auth_token("explicit");
            b
        }),
        ("explicit then env", {
            let mut b = gitea::ReleaseList::configure();
            b.host(HOST)
                .repo_owner("o")
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
}

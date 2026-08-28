//! A5 end-to-end: a blank explicit `auth_token(..)` is treated as unset -- observed on the wire.
//!
//! The motivating pattern is `auth_token(cfg.token.unwrap_or_default()).auth_token_from_env()`,
//! where a missing config value leaves `Some("")` in the slot. Before A5 that blank value both
//! blocked the environment fallback and produced a literal `Authorization: token ` header, turning
//! a working anonymous (or env-authenticated) request into a 401/403. The in-crate tests pin the
//! three pieces separately -- `is_blank_token`, `fill_env_token_if_unset_with`, `apply_auth`,
//! `has_auth_token()` -- but none of them runs a real builder against a real environment and looks
//! at the header that actually goes out, which is where a regression in the wiring between those
//! pieces would show up.
//!
//! It also pins the deliberate ASYMMETRY between the two call orders around a blank token, which no
//! other test states: `auth_token("")` before `auth_token_from_env()` picks the environment up,
//! while `auth_token("")` *after* it overwrites the resolved token and the request goes out
//! anonymous (`auth_token(..)` unconditionally overwrites the slot and clears the env-sourced flag).
//! A real, non-blank explicit token wins in either order -- that symmetry is
//! `tests/auth_token_env_github.rs`'s subject and is unaffected.
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
#![cfg(feature = "github")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use self_update::backends::github;
use self_update::http_client::{HeaderMap, HttpClient, HttpResponse};

/// The value `GH_TOKEN` is set to, distinctive so an assertion cannot match it by accident.
const ENV_TOKEN: &str = "env-token-blank-explicit";

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

/// One test, one process: every blank-token path, on both github builders, ending in a recorded
/// request.
#[test]
fn a_blank_explicit_token_behaves_exactly_like_an_unset_one() {
    // Set before any other thread exists in this process (see the module comment for why).
    // `GH_TOKEN` has precedence over `GITHUB_TOKEN`, so an ambient `GITHUB_TOKEN` on the machine
    // running this cannot change what is resolved.
    unsafe {
        std::env::set_var("GH_TOKEN", ENV_TOKEN);
    }

    // 1. The A5 pattern: a blank explicit token must not block the fallback, and the env token must
    //    reach the wire. `auth_token(cfg.token.unwrap_or_default())` is the shape this comes from.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token("");
    assert!(
        !upd.has_auth_token(),
        "an empty explicit token must not count as configured"
    );
    upd.auth_token_from_env();
    assert!(
        upd.has_auth_token(),
        "the env fallback must not be blocked by the blank value"
    );
    let header = auth_header_of(|client| {
        let _ = upd
            .http_client(client)
            .build()
            .unwrap()
            .get_latest_release();
    });
    assert_eq!(
        header.as_deref(),
        Some("token env-token-blank-explicit"),
        "the env-resolved token must reach the wire over the blank explicit one"
    );

    // 2. The same on the `ReleaseList` builder, whose `auth_token` setter is hand-written per
    //    backend rather than macro-generated, with an all-whitespace value instead of an empty one.
    let mut list = github::ReleaseList::configure();
    list.repo_owner("o").repo_name("r").auth_token("  \t ");
    assert!(!list.has_auth_token());
    list.auth_token_from_env();
    assert!(list.has_auth_token());
    let header = auth_header_of(|client| {
        let _ = list.http_client(client).build().unwrap().fetch();
    });
    assert_eq!(
        header.as_deref(),
        Some("token env-token-blank-explicit"),
        "an all-whitespace explicit token must not block the fallback either"
    );

    // 3. A blank token with no fallback at all must send NO header -- not `Authorization: token `,
    //    which is what a server answers 401/403 to where an anonymous request would have succeeded.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token("   ");
    assert!(!upd.has_auth_token());
    let header = auth_header_of(|client| {
        let _ = upd
            .http_client(client)
            .build()
            .unwrap()
            .get_latest_release();
    });
    assert_eq!(
        header, None,
        "a blank token must produce no Authorization header at all"
    );

    // 4. The asymmetric order: an explicit blank AFTER the env lookup overwrites the resolved token
    //    (the explicit setter is unconditional) and the request goes out anonymous. Pinned so the
    //    behavior cannot flip silently -- it is the one case where the two setters are order
    //    sensitive, and `has_auth_token()` reports it honestly.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token_from_env();
    assert!(upd.has_auth_token(), "the env token was picked up first");
    upd.auth_token("");
    assert!(
        !upd.has_auth_token(),
        "a blank explicit token replaces the env-sourced one and leaves nothing configured"
    );
    let header = auth_header_of(|client| {
        let _ = upd
            .http_client(client)
            .build()
            .unwrap()
            .get_latest_release();
    });
    assert_eq!(
        header, None,
        "auth_token(\"\") after auth_token_from_env() must leave the request anonymous"
    );

    // 5. And the control that makes 1-4 meaningful: a NON-blank explicit token still wins in this
    //    same order, so the behavior above is specific to blankness and not a broken precedence rule.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token_from_env()
        .auth_token("explicit-token");
    let header = auth_header_of(|client| {
        let _ = upd
            .http_client(client)
            .build()
            .unwrap()
            .get_latest_release();
    });
    assert_eq!(
        header.as_deref(),
        Some("token explicit-token"),
        "a real explicit token must still win over the environment in either order"
    );
}

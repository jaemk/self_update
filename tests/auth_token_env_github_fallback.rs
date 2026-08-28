//! Fallback to the **second** declared variable, against a real process environment.
//!
//! # Why this file holds exactly ONE `#[test]`
//!
//! `std::env::set_var` is `unsafe` since the 2024 edition: the environment is process-global, and
//! mutating it while another thread reads it is undefined behavior. Each integration-test file is
//! its own binary and its own process; see the `// SAFETY:` comment at the `set_var` call below for
//! the exact invariant that makes the call sound here. **Do not add a second `#[test]` to this
//! file**; put it in a new single-test file of its own.
//!
//! This file needs its own process because it needs a *different* environment from
//! `auth_token_env_github.rs`: there `GH_TOKEN` is populated and must win, here it is
//! exported-but-blank and must be skipped.
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

/// The github list is the only multi-variable one (`GH_TOKEN`, `GITHUB_TOKEN`), so it is the only
/// place where "fall through to the next name" can be exercised end to end.
///
/// Guards two regressions at once, neither of which any other test would catch:
///
/// 1. A resolver that stopped at the first *present* variable (rather than the first present and
///    non-empty one) would pick the blank `GH_TOKEN` and send `Authorization: token ` -- a
///    guaranteed 401 against a private repo, from an environment that CI scaffolding produces all
///    the time (`GH_TOKEN: ${{ secrets.MAYBE_UNSET }}` exports an empty string).
/// 2. The per-backend in-crate tests only assert the declared const and run literal values through
///    the resolver; nothing there proves the *setter* walks the whole list against the real
///    environment.
#[test]
fn a_blank_first_variable_falls_through_to_the_second() {
    // SAFETY: `std::env::set_var` is sound only while no other thread may read the
    // environment concurrently. What holds here is NOT "this process is single-threaded":
    // libtest keeps its harness thread alive in `recv_timeout` while the body runs on a
    // worker at default concurrency. What holds is that no environment-reading thread
    // exists yet -- this binary contains exactly ONE `#[test]`, and every env write below
    // happens BEFORE the first HTTP client is built. That ordering is load-bearing: a
    // reqwest blocking client spawns a background thread that reads `HTTP_PROXY` /
    // `http_proxy`. So do not add a second `#[test]` here, and do not place a `set_var` /
    // `remove_var` after the first `build()` -- either is a genuine data race, not style.
    // `GH_TOKEN` is exported but whitespace-only: treated as unset.
    unsafe {
        std::env::set_var("GH_TOKEN", "  \t\n");
        std::env::set_var("GITHUB_TOKEN", "  second-var-token\n");
    }

    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0");
    assert!(!upd.has_auth_token(), "nothing is set before the call");
    upd.auth_token_from_env();
    assert!(
        upd.has_auth_token(),
        "a blank GH_TOKEN must not stop the lookup: GITHUB_TOKEN still supplies a token"
    );

    let built = upd.build().expect("an env-sourced token must still build");
    assert_eq!(
        self_update::UpdateConfig::auth_token(&built),
        Some("second-var-token"),
        "the second variable's value must be used, with surrounding whitespace trimmed (an \
         untrimmed value would fail HTTP header encoding at request time)"
    );

    // The same walk on the `ReleaseList` builder, which carries its own copy of the setter.
    // `has_auth_token()` and a successful `build()` both pass under the regression this file
    // exists to catch (a resolver that stopped at the first *present* variable would pick the
    // blank `GH_TOKEN` and still report a token and a buildable config), so the value must be
    // pinned on the wire, exactly as the `Update` half is pinned above.
    let mut list = github::ReleaseList::configure();
    list.repo_owner("o").repo_name("r");
    assert!(!list.has_auth_token());
    list.auth_token_from_env();
    assert!(
        list.has_auth_token(),
        "the ReleaseList builder must walk the same list past the blank first variable"
    );
    let header = auth_header_of(|client| {
        let _ = list.http_client(client).build().unwrap().fetch();
    });
    assert_eq!(
        header.as_deref(),
        Some("token second-var-token"),
        "the second variable's value must reach the wire, not the blank first one"
    );
}

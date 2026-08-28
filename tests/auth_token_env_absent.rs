//! End-to-end check of the documented no-op path: when none of `auth_token_from_env()`'s
//! candidate variables is set, the token is left as it was and the request goes out exactly as
//! before -- observed on the wire, not just through the pure helpers.
//!
//! `impl_auth_token_from_env!`'s rustdoc promises this explicitly, but every other integration
//! binary (`auth_token_env_github.rs`, `_gitlab.rs`, `_gitee.rs`, `_gitea.rs`, ...) sets its
//! candidate variables *first*, so the "nothing set" branch has only ever been exercised through
//! `first_env_token`'s in-crate unit tests, never through a real builder against a real process
//! environment ending in an actual (recorded) HTTP request. A regression that made the no-op path
//! attach some token anyway, or silently drop the request instead of sending it anonymously, would
//! leave every existing integration binary green.
//!
//! # Why this file `remove_var`s instead of relying on a clean environment
//!
//! The process this binary runs in might already have `GH_TOKEN` or `GITHUB_TOKEN` exported (a
//! developer's shell, or a CI runner that injects one for its own purposes). Asserting the no-op
//! path against whatever the ambient environment happens to be would make the test mean different
//! things on different machines -- exactly the failure mode called out for the in-crate
//! `auth_token_from_env_is_available_on_both_builders` test (D10). `remove_var`ing both candidates
//! up front makes "no token is configured" the thing actually under test, independent of the
//! environment cargo was invoked with.
//!
//! # Why this file holds exactly ONE `#[test]`
//!
//! SAFETY: `std::env::set_var` (and `remove_var`) is sound only while no other thread may read the
//! environment concurrently. What holds here is NOT "this process is single-threaded": libtest
//! keeps its harness thread alive in `recv_timeout` while the body runs on a worker at default
//! concurrency. What holds is that no environment-reading thread exists yet -- this binary contains
//! exactly ONE `#[test]`, and every env write below happens BEFORE the first HTTP client is built.
//! That ordering is load-bearing: a reqwest blocking client spawns a background thread that reads
//! `HTTP_PROXY` / `http_proxy`. So do not add a second `#[test]` here, and do not place a `set_var`
//! / `remove_var` after the first `build()` -- either is a genuine data race, not style.
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

/// One test, one process: with neither `GH_TOKEN` nor `GITHUB_TOKEN` present, `auth_token_from_env()`
/// must be a true no-op -- no token configured on the builder, and no `Authorization` header on the
/// wire -- on both the `Update` and `ReleaseList` builders.
#[test]
fn nothing_set_leaves_the_request_unauthenticated() {
    // Remove both candidates before any other thread exists in this process (see the module
    // comment for why this must happen before the first `build()`). Absent variables are not an
    // error for `remove_var`.
    unsafe {
        std::env::remove_var("GH_TOKEN");
        std::env::remove_var("GITHUB_TOKEN");
    }

    // 1. The ReleaseList builder: no candidate is set, so the lookup finds nothing and the flag
    //    the setter records is never turned into a token.
    let mut list = github::ReleaseList::configure();
    list.repo_owner("o").repo_name("r");
    assert!(!list.has_auth_token());
    list.auth_token_from_env();
    assert!(
        !list.has_auth_token(),
        "with neither GH_TOKEN nor GITHUB_TOKEN set, auth_token_from_env() must leave no token \
         configured"
    );
    let header = auth_header_of(|client| {
        let _ = list.http_client(client).build().unwrap().fetch();
    });
    assert_eq!(
        header, None,
        "the request must go out with no Authorization header when nothing was set"
    );

    // 2. The Update builder: same contract through the macro-generated setter.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0");
    assert!(!upd.has_auth_token());
    upd.auth_token_from_env();
    assert!(
        !upd.has_auth_token(),
        "with neither GH_TOKEN nor GITHUB_TOKEN set, auth_token_from_env() must leave no token \
         configured"
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
        "the request must go out with no Authorization header when nothing was set"
    );
}

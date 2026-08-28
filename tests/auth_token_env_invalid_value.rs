//! An environment variable holding a value that is not a legal HTTP header.
//!
//! `auth_token_from_env()`'s rustdoc promises this exact split: the lookup does **not** validate,
//! so a mangled variable surfaces as [`Error::InvalidAuthToken`] at *request* time and **not** from
//! `build()`. Nothing tested that promise end to end -- the in-crate tests cover the invalid-token
//! error with an explicitly-set token, and the env tests only use well-formed values. A `build()`
//! that started validating (or an env path that silently dropped the mangled value) would keep
//! every one of them green while changing the documented contract, and either change moves the
//! failure to a different place than the docs tell the user to look.
//!
//! # Why this file holds exactly ONE `#[test]`
//!
//! `std::env::set_var` is `unsafe` since the 2024 edition: the environment is process-global, and
//! mutating it while another thread reads it (directly, or through libc calls such as
//! `getaddrinfo`) is undefined behavior, and the harness runs the tests of one binary concurrently
//! on many threads. Each integration-test file is its own binary and its own process, so with
//! exactly one test here the `set_var` call happens on the only thread that exists. **Do not add a
//! second `#[test]` to this file**; put it in a new single-test file of its own.
#![cfg(feature = "github")]

use std::sync::Arc;
use std::time::Duration;

use self_update::backends::github;
use self_update::http_client::{HeaderMap, HttpClient, HttpResponse};

/// A transport that must never be reached: the request is refused while its headers are derived.
struct NeverCalled;

impl HttpClient for NeverCalled {
    fn get(
        &self,
        url: &str,
        _headers: &HeaderMap,
        _timeout: Option<Duration>,
    ) -> self_update::Result<Box<dyn HttpResponse>> {
        panic!("a request with an unencodable auth token must not reach the transport: {url}");
    }
}

#[test]
fn an_unencodable_env_token_fails_at_request_time_not_at_build() {
    // A newline in the middle of the value: `trim()` only strips *surrounding* whitespace, so this
    // survives the lookup and is rejected later, when it is rendered into a header value. (CI
    // secrets pasted from a terminal produce exactly this.)
    unsafe {
        std::env::set_var("GH_TOKEN", "ghp_line1\nline2");
    }

    let mut builder = github::Update::configure();
    builder
        .repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token_from_env();
    assert!(
        builder.has_auth_token(),
        "the value is taken as-is: validity is not checked during the lookup"
    );

    let upd = builder
        .http_client(Arc::new(NeverCalled))
        .build()
        .expect("build() must NOT validate the token: the documented failure point is the request");

    let err = upd
        .get_latest_release()
        .expect_err("an unencodable token must fail the request");
    assert!(
        matches!(err, self_update::errors::Error::InvalidAuthToken { .. }),
        "the mangled value must surface as InvalidAuthToken, got: {err:?}"
    );
    // The error must not carry the credential (it is reported to the user, and the value here is
    // one the application author never typed).
    assert!(
        !format!("{err}").contains("ghp_line1"),
        "the error Display must not echo the token: {err}"
    );
}

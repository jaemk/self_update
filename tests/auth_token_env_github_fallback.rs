//! Fallback to the **second** declared variable, against a real process environment.
//!
//! # Why this file holds exactly ONE `#[test]`
//!
//! Same reason as [`auth_token_env.rs`](../auth_token_env.rs): `std::env::set_var` is `unsafe`
//! since the 2024 edition (the environment is process-global, and mutating it while another thread
//! reads it -- directly or through libc calls such as `getaddrinfo` -- is undefined behavior), and
//! the test harness runs the tests of one binary concurrently on many threads. Each integration
//! test file is its own binary, run as its own process, so with exactly one test here the
//! `set_var` calls happen on the only thread that exists. **Do not add a second `#[test]` to this
//! file**; put it in a new single-test file of its own.
//!
//! This file needs its own process because it needs a *different* environment from
//! `auth_token_env.rs`: there `GH_TOKEN` is populated and must win, here it is exported-but-blank
//! and must be skipped.
#![cfg(feature = "github")]

use self_update::backends::github;

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
    // Set before any other thread exists in this process (see the module comment).
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
    let mut list = github::ReleaseList::configure();
    list.repo_owner("o").repo_name("r");
    assert!(!list.has_auth_token());
    list.auth_token_from_env();
    assert!(
        list.has_auth_token(),
        "the ReleaseList builder must walk the same list past the blank first variable"
    );
    list.build()
        .expect("an env-sourced token must still build the release list");
}

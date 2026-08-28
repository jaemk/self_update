//! End-to-end check of `auth_token_from_env()` against a **real** process environment.
//!
//! # Why this file holds exactly ONE `#[test]`
//!
//! `std::env::set_var` is `unsafe` since the 2024 edition: the environment is process-global, and
//! mutating it while another thread reads it (directly, or through libc calls such as `getaddrinfo`
//! or `localtime`) is undefined behavior. Rust's test harness runs the tests of one binary
//! *concurrently on many threads*, so a second `#[test]` in this file could be reading the
//! environment at the moment this one writes it.
//!
//! Each integration-test file is compiled into its **own binary**, and cargo runs the binaries as
//! separate processes. With exactly one test here, the `set_var` call happens on the only thread
//! that exists in this process, before anything else reads the environment — which is what makes it
//! sound. **Do not add a second `#[test]` (or any `#[bench]`/doctest-like helper that runs
//! concurrently) to this file**: put it in the in-crate `#[cfg(test)]` modules, which cover the
//! resolution rules through the pure helpers without touching process env, or in a new
//! single-test file of its own.
#![cfg(feature = "github")]

use self_update::backends::github;

/// One test, one process: sets a GitHub token variable and pins the whole env-token contract on the
/// two github builders.
///
/// Covers what the in-crate unit tests structurally cannot: that the setter really reads the
/// process environment (the unit tests exercise the pure resolver with literal values), that
/// `GH_TOKEN` really beats `GITHUB_TOKEN` when **both** are exported (the in-crate test only proves
/// that of the declared const, not of a live environment), that `has_auth_token()` reports the
/// pickup, and that an explicit `auth_token(..)` still wins in **both** call orders against a
/// genuinely populated environment (the regression that motivated the fallback semantics: the
/// setter pair used to be order-sensitive, so an ambient CI credential could displace the
/// application's own).
#[test]
fn env_token_is_picked_up_and_an_explicit_token_still_wins() {
    // `GH_TOKEN` has precedence over `GITHUB_TOKEN`, matching the `gh` CLI. Both are set, so the
    // precedence is exercised against a real environment rather than asserted off the const list:
    // inside GitHub Actions `GITHUB_TOKEN` is auto-populated, and a reversed list would silently
    // ignore a deliberately exported `GH_TOKEN`. Set before any other thread exists in this process
    // (see the module comment for why that matters).
    unsafe {
        std::env::set_var("GH_TOKEN", "env-token");
        std::env::set_var("GITHUB_TOKEN", "github-token-must-lose");
    }

    // 1. Pickup: with no explicit token, the environment supplies one on both builders.
    let mut upd = github::Update::configure();
    upd.repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0");
    assert!(
        !upd.has_auth_token(),
        "nothing is set before the call: the update would run anonymously"
    );
    upd.auth_token_from_env();
    assert!(
        upd.has_auth_token(),
        "auth_token_from_env() must pick up GH_TOKEN from the process environment"
    );
    let built = upd.build().expect("an env-sourced token must still build");
    assert_eq!(
        self_update::UpdateConfig::auth_token(&built),
        Some("env-token"),
        "the value must come from GH_TOKEN, not from the lower-precedence GITHUB_TOKEN"
    );

    let mut list = github::ReleaseList::configure();
    list.repo_owner("o").repo_name("r");
    assert!(!list.has_auth_token());
    list.auth_token_from_env();
    assert!(
        list.has_auth_token(),
        "the ReleaseList builder must read the same variable"
    );
    list.build()
        .expect("an env-sourced token must still build the release list");

    // 2. Precedence, order 1: env lookup first, then an explicit token.
    let env_then_explicit = github::Update::configure()
        .repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token_from_env()
        .auth_token("explicit")
        .build()
        .unwrap();

    // 3. Precedence, order 2: explicit token first, then the env lookup. This is the direction that
    //    used to lose: the env value overwrote the application's own credential.
    let explicit_then_env = github::Update::configure()
        .repo_owner("o")
        .repo_name("r")
        .bin_name("app")
        .current_version("0.1.0")
        .auth_token("explicit")
        .auth_token_from_env()
        .build()
        .unwrap();

    for upd in [env_then_explicit, explicit_then_env] {
        // Read the resolved token through the public `UpdateConfig` accessor, which is what the
        // request path itself uses, rather than reaching into crate internals.
        assert_eq!(
            self_update::UpdateConfig::auth_token(&upd),
            Some("explicit"),
            "an explicit auth_token(..) must win over the environment in either call order"
        );
    }
}

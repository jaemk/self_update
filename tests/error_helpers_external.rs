//! External-crate regression tests for `self_update::errors::Error`.
//!
//! Integration tests live in a separate crate from `self_update`, so they exercise the public
//! API under the same `#[non_exhaustive]` restrictions that downstream consumers face.
//!
//! ## What is and is not testable here
//!
//! Every struct variant is annotated `#[non_exhaustive]` (`Unauthorized`, `HttpStatus`,
//! `NotFound`, `ChecksumMismatch`, `InvalidAssetName`, …) and **cannot be constructed with a
//! struct literal from outside the crate**. Attempting:
//!
//! ```ignore
//! // compile error: cannot create non-exhaustive struct with explicit field values from outside
//! let _ = self_update::errors::Error::Unauthorized { status: 401, url: "u".into() };
//! ```
//!
//! fails at compile time, which is the enforcement the attribute is supposed to provide.
//! To write a passing test that fails when the attribute is removed would require `trybuild`
//! (compile-fail tests). Without it, enforcement of the variant-level attribute is acceptably
//! untestable from a passing-test perspective; the in-crate tests in `src/errors.rs` pin the
//! observable behaviour (Display strings, helpers, source() return values).
//!
//! What IS testable here:
//! - The public constructors (`Error::http_status_error`, `Error::no_release_found`,
//!   `Error::verification_rejected`, …) build the variants a downstream crate needs to return.
//! - Tuple variants that remain constructable from outside: `Io`, `Aborted`, …
//! - The enum-level `#[non_exhaustive]` forces a wildcard in any downstream `match`.
//! - Error propagation through an injected `HttpClient` (error path not covered elsewhere).

#![cfg(feature = "github")]

use std::sync::Arc;
use std::time::Duration;

use self_update::errors::Error;
use self_update::http_client::{HeaderMap, HttpClient, HttpResponse};
use std::error::Error as StdError;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A transport that immediately returns an `Io` error for every request.
/// `Error::Io` is constructable from outside because `Io` is a plain tuple variant
/// with no `#[non_exhaustive]` annotation on the variant itself.
struct IoErrorClient;

impl HttpClient for IoErrorClient {
    fn get(
        &self,
        _url: &str,
        _headers: &HeaderMap,
        _timeout: Option<Duration>,
    ) -> self_update::Result<Box<dyn HttpResponse>> {
        Err(Error::Io(std::io::Error::other("simulated failure")))
    }
}

/// Build a `HeaderMap` from `(name, value)` pairs — the shape a custom `HttpClient` has in hand
/// when it maps a non-2xx response through `Error::http_status_error_with_headers`.
fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(*name, value.parse().expect("valid header value"));
    }
    map
}

/// The unix timestamp `secs` seconds from now, as the string an `x-ratelimit-reset` /
/// `RateLimit-Reset` header carries. Negative offsets express an already-elapsed window.
fn reset_epoch(offset_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after the unix epoch")
        .as_secs() as i64;
    (now + offset_secs).to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// `Error::NotFound` is `#[non_exhaustive]` on the variant, so a downstream crate builds it via
// the `http_status_error` constructor (404 maps to `NotFound`). This pins the constructor
// contract and verifies `http_status()` / `url()` / `source()` from an external-crate
// perspective.
#[test]
fn not_found_constructable_and_helpers_correct_from_outside() {
    let err = Error::http_status_error(404, "https://example.com/missing");
    assert!(matches!(err, Error::NotFound { .. }));
    assert_eq!(err.http_status(), Some(404));
    assert_eq!(err.url(), Some("https://example.com/missing"));
    // NotFound is field-only: no chained source.
    assert!(
        err.source().is_none(),
        "NotFound must not expose a chained source()"
    );
    let shown = err.to_string();
    assert!(shown.starts_with("NotFoundError: "), "got: {shown}");
}

// `Error::verification_rejected` builds the rejection a `verify_binary` hook returns; the
// `VerificationRejected` variant itself is `#[non_exhaustive]` and not literal-constructable.
#[test]
fn verification_rejected_constructable_from_outside() {
    let err = Error::verification_rejected("bad signature");
    assert!(matches!(err, Error::VerificationRejected { .. }));
    let shown = err.to_string();
    assert!(shown.contains("bad signature"), "got: {shown}");
    assert_eq!(err.http_status(), None);
    assert_eq!(err.url(), None);
}

// `Error::no_release_found` / `no_release_found_for_target` build the two shapes of
// `NoReleaseFound` (which is `#[non_exhaustive]` and not literal-constructable) without the
// caller spelling out an `Option`.
#[test]
fn no_release_found_constructors_from_outside() {
    let plain = Error::no_release_found();
    assert!(matches!(plain, Error::NoReleaseFound { .. }));

    // `impl Into<String>`: a `&str` and a `format!` product both work.
    let scoped = Error::no_release_found_for_target("x86_64-unknown-linux-gnu");
    let shown = scoped.to_string();
    assert!(shown.contains("x86_64-unknown-linux-gnu"), "got: {shown}");
    let _ = Error::no_release_found_for_target(format!("{}-msvc", "x86_64"));
}

// `Error::missing_asset_field` accepts a dynamic field path, not just a `&'static str`.
#[test]
fn missing_asset_field_accepts_dynamic_paths_from_outside() {
    let idx = 2;
    let err = Error::missing_asset_field(format!("assets[{idx}].url"));
    assert!(matches!(err, Error::MissingAssetField { .. }));
    let shown = err.to_string();
    assert!(shown.contains("assets[2].url"), "got: {shown}");
}

// `Error::checksum_mismatch` builds the `ChecksumMismatch` variant, which is
// `#[non_exhaustive]` and otherwise unconstructable from outside the crate.
#[test]
fn checksum_mismatch_constructable_from_outside() {
    let err = Error::checksum_mismatch("aa11", "bb22");
    assert!(matches!(err, Error::ChecksumMismatch { .. }));
    let shown = err.to_string();
    assert!(
        shown.contains("aa11") && shown.contains("bb22"),
        "Display must carry both digests, got: {shown}"
    );
    assert_eq!(err.http_status(), None);
    assert_eq!(err.url(), None);
}

// `Error::transport` builds the `Transport` variant from either an error value or a message
// string, so a custom `HttpClient` can report a failed request without spelling out the
// `Box<dyn Error + Send + Sync>` conversion.
#[test]
fn transport_constructor_from_outside() {
    // From an error value: source() chains to it.
    let err = Error::transport(std::io::Error::other("connection reset"));
    assert!(matches!(err, Error::Transport(_)));
    let src = err.source().expect("Error::transport must chain source()");
    assert!(src.to_string().contains("connection reset"), "got: {src}");
    let shown = err.to_string();
    assert!(shown.starts_with("TransportError: "), "got: {shown}");

    // From a message string.
    let err = Error::transport("proxy refused the request");
    assert!(matches!(err, Error::Transport(_)));
    assert!(
        err.to_string().contains("proxy refused the request"),
        "got: {err}"
    );
}

// `Error::Io` wraps `std::io::Error` which itself implements `std::error::Error`.
// The `source()` chain works end-to-end from outside the crate.
#[test]
fn io_error_source_accessible_from_outside() {
    let err = Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "denied",
    ));
    let src = err.source().expect("Error::Io must have a source");
    // The source is the inner io::Error; its message is accessible.
    assert!(
        src.to_string().contains("denied"),
        "source must carry the inner io message, got: {}",
        src
    );
    assert_eq!(err.http_status(), None);
    assert_eq!(err.url(), None);
}

// A downstream `match` against `Error` MUST include a wildcard arm because `Error` is
// `#[non_exhaustive]` at the enum level. This test would fail to compile if the wildcard
// arm were removed, pinning the enum-level non-exhaustive contract from outside the crate.
#[test]
fn error_enum_match_requires_wildcard_arm() {
    fn classify(err: &Error) -> &'static str {
        match err {
            Error::NotFound { .. } => "not-found",
            Error::Aborted => "aborted",
            // Required: Error is #[non_exhaustive], so new variants can be added without a
            // breaking change. Omitting this arm is a compile error.
            _ => "other",
        }
    }

    assert_eq!(classify(&Error::http_status_error(404, "u")), "not-found");
    assert_eq!(classify(&Error::Aborted), "aborted");
    assert_eq!(
        classify(&Error::Io(std::io::Error::other("x"))),
        "other",
        "Io and any future variants fall through to the wildcard"
    );
}

// An error returned by a custom `HttpClient` implementation propagates through the backend
// and is received by the caller as an `Err`. This is the error path of the transport-injection
// contract; the success path is covered by `custom_transport.rs`. Without this test, a
// regression that swallows or transforms errors from injected transports would go unnoticed.
#[test]
fn injected_transport_error_propagates_through_backend() {
    let result = self_update::backends::github::ReleaseList::configure()
        .repo_owner("o")
        .repo_name("r")
        .http_client(Arc::new(IoErrorClient))
        .build()
        .unwrap()
        .fetch();

    assert!(result.is_err(), "fetch must fail when the transport errors");
    match result.unwrap_err() {
        Error::Io(_) => {} // expected: the Io error returned by the transport
        other => panic!("expected Error::Io, got {:?}", other),
    }
}

/// A transport that always fails with a fixed `Error` and counts how many times it was called.
/// `error` is cloned per-call via the supplied closure so both a header-aware `RateLimited` and a
/// plain `HttpStatus` can be pinned without hand-rolling `Clone` for `Error`.
struct CountingErrorClient<F> {
    calls: std::sync::atomic::AtomicUsize,
    make_error: F,
}

impl<F> CountingErrorClient<F>
where
    F: Fn() -> Error + Send + Sync,
{
    fn new(make_error: F) -> Self {
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            make_error,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl<F> HttpClient for CountingErrorClient<F>
where
    F: Fn() -> Error + Send + Sync,
{
    fn get(
        &self,
        _url: &str,
        _headers: &HeaderMap,
        _timeout: Option<Duration>,
    ) -> self_update::Result<Box<dyn HttpResponse>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err((self.make_error)())
    }
}

// The downstream half of the retry-short-circuit contract (`src/backends/mod.rs:1768`, `:1790`
// pin the in-crate half over a real loopback stub): a custom `HttpClient` that returns
// `Error::RateLimited` must see exactly ONE call, no matter the configured `.retries(..)` budget —
// the crate does not call the injected transport again. This is pinned from outside the crate
// because a custom-transport author can only observe this contract through the public
// `HttpClient` seam, not through the in-crate unit tests.
#[test]
fn injected_rate_limited_error_is_not_retried() {
    let url = "https://example.com/releases";
    let client = Arc::new(CountingErrorClient::new(move || {
        Error::http_status_error_with_headers(429, url, &HeaderMap::new())
    }));

    let result = self_update::backends::github::ReleaseList::configure()
        .repo_owner("o")
        .repo_name("r")
        .http_client(client.clone())
        .retries(3)
        .build()
        .unwrap()
        .fetch();

    assert!(
        matches!(result, Err(Error::RateLimited { status: 429, .. })),
        "expected Error::RateLimited, got {:?}",
        result
    );
    assert_eq!(
        client.calls(),
        1,
        "a RateLimited error must end the retry loop immediately: exactly one call to the \
         injected transport, even with retries = 3"
    );
}

// The control for the test above: every other error still consumes the whole retry budget
// (1 initial attempt + `retries` retries). Without this pair, a transport-injection regression
// that short-circuited on EVERY error (not just `RateLimited`) would pass the test above.
#[test]
fn injected_non_rate_limited_error_still_consumes_the_retry_budget() {
    let url = "https://example.com/releases";
    let client = Arc::new(CountingErrorClient::new(move || {
        Error::http_status_error(500, url)
    }));

    let result = self_update::backends::github::ReleaseList::configure()
        .repo_owner("o")
        .repo_name("r")
        .http_client(client.clone())
        .retries(3)
        .build()
        .unwrap()
        .fetch();

    assert!(
        matches!(result, Err(Error::HttpStatus { status: 500, .. })),
        "expected Error::HttpStatus, got {:?}",
        result
    );
    assert_eq!(
        client.calls(),
        4,
        "a non-RateLimited error must consume the whole retry budget: 1 initial attempt + 3 \
         retries = 4 calls to the injected transport"
    );
}

// A4 x D7: the interaction that motivated pairing the two fixes. Before A4, a downstream
// `HttpClient` answering `403 + Retry-After: 0` with no other rate-limit signal classified as
// `Error::RateLimited` (a zero-second wait) and the retry loop's short-circuit ended the loop
// after exactly one call -- silently discarding the retry budget for what is, underneath, a bare
// authorization failure. After A4, `parse_retry_after` treats a zero delay as no signal, so this
// response classifies as `Error::Unauthorized` instead, and `Unauthorized` is not short-circuited:
// it must consume the whole configured retry budget exactly like the plain-500 control above. A
// regression that keyed the short-circuit on "this looked like a rate-limit response" (status +
// header presence) rather than on the `RateLimited` variant itself would pass A4's own unit tests
// (which only check the returned variant, not what the retry loop then does with it) while still
// silently eating the retry budget here.
#[test]
fn injected_403_with_zero_retry_after_is_unauthorized_and_still_consumes_the_retry_budget() {
    let url = "https://example.com/releases";
    let client = Arc::new(CountingErrorClient::new(move || {
        Error::http_status_error_with_headers(403, url, &headers(&[("retry-after", "0")]))
    }));

    let result = self_update::backends::github::ReleaseList::configure()
        .repo_owner("o")
        .repo_name("r")
        .http_client(client.clone())
        .retries(3)
        .build()
        .unwrap()
        .fetch();

    assert!(
        matches!(result, Err(Error::Unauthorized { status: 403, .. })),
        "a 403 with Retry-After: 0 and no other rate-limit signal must classify as Unauthorized, \
         got {:?}",
        result
    );
    assert_eq!(
        client.calls(),
        4,
        "Unauthorized is not short-circuited: 1 initial attempt + 3 retries = 4 calls to the \
         injected transport, exactly as a non-rate-limited 500 would consume"
    );
}

// ---------------------------------------------------------------------------
// `Error::RateLimited`: the header-aware constructor, the accessors, and the back-off
// ---------------------------------------------------------------------------

// `Error::RateLimited` is `#[non_exhaustive]`, so a downstream `HttpClient` can only produce it
// through `http_status_error_with_headers`. Pin every shape that must trigger it from outside the
// crate: a spent primary quota (403 + remaining 0), GitHub's *secondary* limit (403 + `Retry-After`
// with quota still remaining), gitlab's un-prefixed `RateLimit-Remaining` spelling, and a bare 429
// with no quota headers at all (what proxies/CDNs return). A custom transport that gets any of
// these wrong reports "wait" as "your credentials are bad", or vice versa.
#[test]
fn http_status_error_with_headers_builds_rate_limited_for_each_triggering_shape() {
    let spent_primary = Error::http_status_error_with_headers(
        403,
        "https://api.github.com/repos/o/r/releases",
        &headers(&[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", &reset_epoch(600)),
        ]),
    );
    assert!(
        matches!(spent_primary, Error::RateLimited { status: 403, .. }),
        "403 + spent quota must be RateLimited, got {spent_primary:?}"
    );

    let secondary = Error::http_status_error_with_headers(
        403,
        "https://api.github.com/repos/o/r/releases",
        &headers(&[("x-ratelimit-remaining", "57"), ("retry-after", "60")]),
    );
    assert!(
        matches!(secondary, Error::RateLimited { status: 403, .. }),
        "403 + Retry-After (secondary limit, quota remaining) must be RateLimited, got \
         {secondary:?}"
    );

    let gitlab = Error::http_status_error_with_headers(
        403,
        "https://gitlab.com/api/v4/projects/o%2Fr/releases",
        &headers(&[("RateLimit-Remaining", "0")]),
    );
    assert!(
        matches!(gitlab, Error::RateLimited { status: 403, .. }),
        "gitlab's un-prefixed spent-quota spelling must be RateLimited, got {gitlab:?}"
    );

    let bare_429 = Error::http_status_error_with_headers(
        429,
        "https://example.com/releases",
        &HeaderMap::new(),
    );
    assert!(
        matches!(bare_429, Error::RateLimited { status: 429, .. }),
        "a 429 with no quota headers at all must still be RateLimited, got {bare_429:?}"
    );
}

// The negative half of the same contract, from outside: the broadened rule must not have become
// "any 403, or any status with rate-limit headers, is a rate limit". A bare 403 and a 403 reporting
// quota *remaining* are genuine authorization failures; a 401, a 404, and a 500 are untouched by
// quota headers. Getting this wrong tells a caller to sit and wait out a wrong token.
#[test]
fn http_status_error_with_headers_does_not_rate_limit_other_shapes() {
    let bare_403 =
        Error::http_status_error_with_headers(403, "https://example.com/x", &HeaderMap::new());
    assert!(
        matches!(bare_403, Error::Unauthorized { status: 403, .. }),
        "a bare 403 must stay Unauthorized, got {bare_403:?}"
    );

    let quota_remains = Error::http_status_error_with_headers(
        403,
        "https://example.com/x",
        &headers(&[
            ("x-ratelimit-remaining", "57"),
            ("x-ratelimit-reset", &reset_epoch(600)),
        ]),
    );
    assert!(
        matches!(quota_remains, Error::Unauthorized { status: 403, .. }),
        "a 403 with quota remaining must stay Unauthorized, got {quota_remains:?}"
    );

    // Only 403 and 429 are rate-limitable: a 401 carrying a spent quota is still a bad credential.
    let unauth_401 = Error::http_status_error_with_headers(
        401,
        "https://example.com/x",
        &headers(&[("x-ratelimit-remaining", "0"), ("retry-after", "60")]),
    );
    assert!(
        matches!(unauth_401, Error::Unauthorized { status: 401, .. }),
        "a 401 must stay Unauthorized regardless of quota headers, got {unauth_401:?}"
    );

    let not_found = Error::http_status_error_with_headers(
        404,
        "https://example.com/x",
        &headers(&[("x-ratelimit-remaining", "0"), ("retry-after", "60")]),
    );
    assert!(
        matches!(not_found, Error::NotFound { .. }),
        "a 404 must stay NotFound regardless of quota headers, got {not_found:?}"
    );

    let server_error = Error::http_status_error_with_headers(
        500,
        "https://example.com/x",
        &headers(&[("x-ratelimit-remaining", "0"), ("retry-after", "60")]),
    );
    assert!(
        matches!(server_error, Error::HttpStatus { status: 500, .. }),
        "a 500 must stay HttpStatus regardless of quota headers, got {server_error:?}"
    );
}

// The upgrade-facing statement of the behaviour change: code that used to detect a spent GitHub
// quota by matching `Error::Unauthorized { status: 403, .. }` now silently stops matching, because
// that response classifies as `RateLimited`. This is the one break a downstream consumer hits
// without a compile error, so it gets an executable statement of the new contract rather than only
// a line in the migration notes.
#[test]
fn a_spent_quota_403_is_no_longer_matched_as_unauthorized() {
    let err = Error::http_status_error_with_headers(
        403,
        "https://api.github.com/repos/o/r/releases",
        &headers(&[("x-ratelimit-remaining", "0")]),
    );
    assert!(
        !matches!(err, Error::Unauthorized { .. }),
        "a rate-limited 403 must no longer match Unauthorized (the 0.x/1.0 behaviour), got {err:?}"
    );
    assert!(
        matches!(err, Error::RateLimited { status: 403, .. }),
        "it must match RateLimited instead, got {err:?}"
    );
    // The status itself is unchanged, so `http_status() == Some(403)` remains a valid way to detect
    // "the server said 403" across both variants.
    assert_eq!(err.http_status(), Some(403));
}

// `http_status()` / `url()` / `source()` / `Display` on the new variant, from a downstream
// position: the accessors are the documented way to read an HTTP error without matching a
// `#[non_exhaustive]` variant, and `RateLimited` had to be added to each of them by hand.
#[test]
fn rate_limited_accessors_and_display_from_outside() {
    let err = Error::http_status_error_with_headers(
        429,
        "https://api.github.com/repos/o/r/releases",
        &headers(&[("retry-after", "60")]),
    );
    assert_eq!(err.http_status(), Some(429));
    assert_eq!(err.url(), Some("https://api.github.com/repos/o/r/releases"));
    assert!(
        err.source().is_none(),
        "RateLimited is field-only: no chained source()"
    );
    let shown = err.to_string();
    assert!(shown.starts_with("RateLimitedError: "), "got: {shown}");
    assert!(
        shown.contains("https://api.github.com/repos/o/r/releases"),
        "Display must name the request URL, got: {shown}"
    );
    assert!(
        shown.contains("60"),
        "Display must render the known wait, got: {shown}"
    );

    // A downstream `match` still needs its wildcard: `RateLimited` is reached through the enum's
    // `#[non_exhaustive]` surface like every other variant.
    let described = match &err {
        Error::RateLimited { status, .. } => format!("rate-limited {status}"),
        _ => "other".to_string(),
    };
    assert_eq!(described, "rate-limited 429");
}

// `rate_limit_delay()` is the whole point of the variant for a consumer ("how long do I sleep?"),
// and its precedence is not observable from the fields alone. Pin all five outcomes from outside:
// Retry-After wins over reset_at, Retry-After alone, a future reset_at alone, an elapsed reset_at
// (`None`, not a panic and not a zero sleep that burns more quota), and no signal at all.
#[test]
fn rate_limit_delay_precedence_from_outside() {
    // Both signals: the explicit Retry-After wins over the derived reset_at wait.
    let both = Error::http_status_error_with_headers(
        429,
        "https://example.com/x",
        &headers(&[
            ("retry-after", "30"),
            ("x-ratelimit-reset", &reset_epoch(3600)),
        ]),
    );
    assert_eq!(
        both.rate_limit_delay(),
        Some(Duration::from_secs(30)),
        "Retry-After must take precedence over the reset instant"
    );

    // Retry-After only (GitHub's secondary limit shape).
    let retry_only = Error::http_status_error_with_headers(
        403,
        "https://example.com/x",
        &headers(&[("retry-after", "45")]),
    );
    assert_eq!(retry_only.rate_limit_delay(), Some(Duration::from_secs(45)));

    // A future reset_at only (GitHub's *primary* limit sends no Retry-After): the wait is derived
    // from the reset instant, so it must be a real positive duration, not `None` and not zero.
    let reset_only = Error::http_status_error_with_headers(
        403,
        "https://example.com/x",
        &headers(&[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", &reset_epoch(600)),
        ]),
    );
    let wait = reset_only
        .rate_limit_delay()
        .expect("a future reset instant must yield a wait");
    assert!(
        wait > Duration::from_secs(540) && wait <= Duration::from_secs(600),
        "the derived wait must be ~600s (reset minus now), got {wait:?}"
    );

    // An already-elapsed window: `None` ("retry when you like"), never a panic from a reversed
    // duration_since and never a bogus wait.
    let elapsed = Error::http_status_error_with_headers(
        403,
        "https://example.com/x",
        &headers(&[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", &reset_epoch(-3600)),
        ]),
    );
    assert!(
        matches!(elapsed, Error::RateLimited { .. }),
        "an elapsed window is still a rate limit, got {elapsed:?}"
    );
    assert_eq!(
        elapsed.rate_limit_delay(),
        None,
        "an elapsed reset instant yields no wait"
    );

    // Neither signal (bare 429), and a non-RateLimited variant: both `None`.
    let bare =
        Error::http_status_error_with_headers(429, "https://example.com/x", &HeaderMap::new());
    assert_eq!(bare.rate_limit_delay(), None);
    assert_eq!(
        Error::http_status_error(404, "https://example.com/x").rate_limit_delay(),
        None,
        "rate_limit_delay must be None for a non-RateLimited variant"
    );
    assert_eq!(Error::Aborted.rate_limit_delay(), None);
}

// The 24h ceiling on server-supplied waits, observed from where it matters: a caller asking
// `rate_limit_delay()` how long to sleep. The values come from the response, so an absurd one must
// resolve to `None` (fall back to your own policy) rather than a week-long sleep that would park an
// update channel — and the boundary value itself must still be honoured.
#[test]
fn rate_limit_delay_rejects_absurd_server_supplied_waits_from_outside() {
    // 7 days of Retry-After on a 429: still a rate limit (429 always is), but no usable wait.
    let absurd_retry = Error::http_status_error_with_headers(
        429,
        "https://example.com/x",
        &headers(&[("retry-after", "604800")]),
    );
    assert!(matches!(
        absurd_retry,
        Error::RateLimited { status: 429, .. }
    ));
    assert_eq!(
        absurd_retry.rate_limit_delay(),
        None,
        "an over-ceiling Retry-After must yield no wait, not a week-long sleep"
    );

    // A reset instant 30 days out is rejected the same way.
    let absurd_reset = Error::http_status_error_with_headers(
        429,
        "https://example.com/x",
        &headers(&[("x-ratelimit-reset", &reset_epoch(30 * 24 * 3600))]),
    );
    assert_eq!(
        absurd_reset.rate_limit_delay(),
        None,
        "an over-ceiling reset instant must yield no wait"
    );

    // Exactly 24h is at the ceiling, not past it, and is still honoured.
    let at_ceiling = Error::http_status_error_with_headers(
        429,
        "https://example.com/x",
        &headers(&[("retry-after", "86400")]),
    );
    assert_eq!(
        at_ceiling.rate_limit_delay(),
        Some(Duration::from_secs(86400)),
        "a Retry-After exactly at the 24h ceiling must be honoured"
    );

    // And on a 403 the rejected value is no signal at all, so the response stays an authorization
    // failure instead of becoming a RateLimited carrying no wait.
    let over_ceiling_403 = Error::http_status_error_with_headers(
        403,
        "https://example.com/x",
        &headers(&[("retry-after", "604800")]),
    );
    assert!(
        matches!(over_ceiling_403, Error::Unauthorized { status: 403, .. }),
        "an over-ceiling Retry-After must not promote a 403 to RateLimited, got \
         {over_ceiling_403:?}"
    );
}

// The header-blind constructor is the fallback for a custom transport that cannot see response
// headers, and it must agree with the header-aware one about what a 429 *means*: RFC 6585 defines
// the status itself as rate limiting, so a 429 is `RateLimited` from either constructor. It used to
// fall through to `HttpStatus` here, contradicting the `HttpStatus` variant docs ("429 never lands
// here") for exactly the downstream `HttpClient` those docs are written for. The headers only supply
// the *wait*, so this form carries neither wait field and `rate_limit_delay()` is `None`.
#[test]
fn header_blind_http_status_error_still_classifies_429_as_rate_limited() {
    let err = Error::http_status_error(429, "https://example.com/x");
    let Error::RateLimited {
        status,
        reset_at,
        retry_after,
        ..
    } = err
    else {
        panic!("the header-blind constructor must classify a 429 as RateLimited, got {err:?}");
    };
    assert_eq!(status, 429);
    assert_eq!(
        reset_at, None,
        "a header-blind 429 has no reset instant to carry"
    );
    assert_eq!(
        retry_after, None,
        "a header-blind 429 has no Retry-After to carry"
    );
    assert_eq!(
        err.rate_limit_delay(),
        None,
        "with neither wait field there is no known wait"
    );
    assert_eq!(err.http_status(), Some(429));

    // Only 429 moved: a bare 403 still needs a header to distinguish a spent quota from a bad
    // credential, so the header-blind form keeps reporting it as an authorization failure.
    let err = Error::http_status_error(403, "https://example.com/x");
    assert!(
        matches!(err, Error::Unauthorized { status: 403, .. }),
        "a header-blind 403 must stay Unauthorized, got {err:?}"
    );
    let err = Error::http_status_error(401, "https://example.com/x");
    assert!(
        matches!(err, Error::Unauthorized { status: 401, .. }),
        "a header-blind 401 must stay Unauthorized, got {err:?}"
    );
}

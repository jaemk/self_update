//! External-crate regression tests for `self_update::errors::Error`.
//!
//! Integration tests live in a separate crate from `self_update`, so they exercise the public
//! API under the same `#[non_exhaustive]` restrictions that downstream consumers face.
//!
//! ## What is and is not testable here
//!
//! The three struct variants newly annotated `#[non_exhaustive]` — `Unauthorized`, `HttpStatus`,
//! `InvalidAssetName` — **cannot be constructed from outside the crate**. Attempting:
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
//! - Variants that are constructable from outside: `NotFound`, `Io`, `Aborted`, `Config`, …
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// `Error::NotFound` is not `#[non_exhaustive]` on the variant, so it can be constructed
// and exhaustively destructured from outside. This pins the external constructability contract
// and verifies `http_status()` / `url()` / `source()` from an external-crate perspective.
#[test]
fn not_found_constructable_and_helpers_correct_from_outside() {
    let err = Error::NotFound {
        url: "https://example.com/missing".to_string(),
    };
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

    assert_eq!(classify(&Error::NotFound { url: "u".into() }), "not-found");
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

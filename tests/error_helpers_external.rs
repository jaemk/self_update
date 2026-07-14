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

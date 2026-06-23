/*!
Error type, conversions, and macros

*/
#[cfg(feature = "archive-zip")]
use zip::result::ZipError;

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(feature = "signatures")]
use zipsign_api::ZipsignError;

/// The crate's single public error type.
///
/// ## Matching on variants
///
/// `Error` is `#[non_exhaustive]`, so a `match` must include a wildcard arm. For programmatic
/// decisions, prefer `http_status()` and `url()` over matching on the Display string — the
/// Display strings are human-facing and may change between minor releases.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Used as a catch-most for when the program fails to update.
    Update(String),
    /// The downloaded artifact's checksum did not match the expected digest.
    ///
    /// `expected` is the configured digest; `computed` is the one actually produced from the
    /// downloaded file. Both are hex-encoded lowercase digests.
    ChecksumMismatch {
        /// The expected digest (from the configured `Checksum`), hex-encoded.
        expected: String,
        /// The digest produced from the downloaded file, hex-encoded.
        computed: String,
    },
    /// The user declined the interactive confirmation prompt.
    ///
    /// Returned when `no_confirm` is `false` (the default) and the user answers anything other
    /// than `y` / `Y` / Enter at the "Do you want to continue?" prompt.
    Aborted,
    /// A request completed and returned HTTP 404 (resource not found).
    ///
    /// `url` is the request URL that produced the 404.
    NotFound {
        /// The URL whose response was HTTP 404.
        url: String,
    },
    /// A request completed and returned HTTP 401 or 403 (not authorized).
    ///
    /// `status` is the exact HTTP status code (401 or 403). `url` is the request URL.
    Unauthorized {
        /// The HTTP status code (401 or 403).
        status: u16,
        /// The URL whose response was this status.
        url: String,
    },
    /// A request completed and returned a non-2xx status other than 404, 401, or 403.
    ///
    /// `status` is the HTTP status code. `url` is the request URL.
    HttpStatus {
        /// The HTTP status code.
        status: u16,
        /// The URL whose response was this status.
        url: String,
    },
    /// If there is an issue with the most recent release (such as no
    /// binary for the current platform), this error is returned.
    Release(String),
    /// Used when a there is an error with setting up the configuration
    /// for a repository archive. An example would be failing to provide the username a
    /// repository archive is under.
    Config(String),
    /// A wrapper over a `std::io::Error`.
    Io(std::io::Error),
    /// A wrapper over a zip archive error (`archive-zip`).
    ///
    /// The concrete error is boxed so that the public API does not change when the underlying
    /// `zip` implementation evolves. Use [`std::error::Error::source`] to inspect the underlying
    /// error.
    #[cfg(feature = "archive-zip")]
    Zip(Box<dyn std::error::Error + Send + Sync>),
    /// A wrapper over a `serde_json::Error`.
    ///
    /// The concrete error is boxed so that the public API does not change when the underlying
    /// `serde_json` implementation evolves. Use [`std::error::Error::source`] to inspect the
    /// underlying error.
    Json(Box<dyn std::error::Error + Send + Sync>),
    /// The request could not be completed (connection/TLS/timeout/transport failure).
    ///
    /// The concrete error is boxed so that the public API does not change when the
    /// `reqwest` / `ureq` feature selection changes. Use [`std::error::Error::source`]
    /// to inspect the underlying error.
    Transport(Box<dyn std::error::Error + Send + Sync>),
    /// A wrapper over a `semver::Error`.
    ///
    /// The concrete error is boxed so that the public API does not change when the underlying
    /// `semver` implementation evolves. Use [`std::error::Error::source`] to inspect the
    /// underlying error.
    SemVer(Box<dyn std::error::Error + Send + Sync>),
    /// Used when the `archive-zip` feature is not enabled.
    ArchiveNotEnabled(String),
    /// Used when the repository archive does not contain any signatures to verify with.
    #[cfg(feature = "signatures")]
    NoSignatures(crate::ArchiveKind),
    /// A wrapper over a signature-verification error (`signatures`).
    ///
    /// The concrete error is boxed so that the public API surface does not depend on the
    /// signing implementation's internal error types. Use [`std::error::Error::source`]
    /// to inspect the underlying error.
    #[cfg(feature = "signatures")]
    Signature(Box<dyn std::error::Error + Send + Sync>),
    /// Used when the path generated to store the repository archive
    /// contains non-UTF8 characters.
    #[cfg(feature = "signatures")]
    SignatureNonUTF8,
    /// A wrapper over the errors that can occur while signing S3 requests (`s3-auth`).
    ///
    /// The concrete error is boxed so that the public API surface does not depend on the
    /// signing implementation's internal error types. Use [`std::error::Error::source`]
    /// to inspect the underlying error.
    #[cfg(feature = "s3-auth")]
    S3Auth(Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// The HTTP status code if this error came from a completed non-2xx response
    /// (`NotFound` => 404, `Unauthorized`/`HttpStatus` => their code); `None` otherwise.
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Error::NotFound { .. } => Some(404),
            Error::Unauthorized { status, .. } => Some(*status),
            Error::HttpStatus { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The URL of the request that failed, for the HTTP error variants
    /// (`NotFound`/`Unauthorized`/`HttpStatus`); `None` otherwise.
    pub fn url(&self) -> Option<&str> {
        match self {
            Error::NotFound { url } => Some(url.as_str()),
            Error::Unauthorized { url, .. } => Some(url.as_str()),
            Error::HttpStatus { url, .. } => Some(url.as_str()),
            _ => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use Error::*;
        match self {
            Update(s) => write!(f, "UpdateError: {}", s),
            ChecksumMismatch { expected, computed } => write!(
                f,
                "ChecksumMismatchError: checksum mismatch (expected {}, computed {})",
                expected, computed
            ),
            Aborted => write!(f, "AbortedError: the update was not confirmed"),
            NotFound { url } => write!(f, "NotFoundError: no resource found at {} (HTTP 404)", url),
            Unauthorized { status, url } => write!(
                f,
                "UnauthorizedError: request to {} was not authorized (HTTP {})",
                url, status
            ),
            HttpStatus { status, url } => write!(
                f,
                "HttpStatusError: request to {} failed with status {}",
                url, status
            ),
            Release(s) => write!(f, "ReleaseError: {}", s),
            Config(s) => write!(f, "ConfigError: {}", s),
            Io(e) => write!(f, "IoError: {}", e),
            Json(e) => write!(f, "JsonError: {}", e),
            Transport(e) => write!(f, "TransportError: {}", e),
            SemVer(e) => write!(f, "SemVerError: {}", e),
            #[cfg(feature = "archive-zip")]
            Zip(e) => write!(f, "ZipError: {}", e),
            ArchiveNotEnabled(s) => write!(
                f,
                "ArchiveNotEnabledError: Archive extension '{}' not supported, please enable 'archive-{}' feature!",
                s, s
            ),
            #[cfg(feature = "signatures")]
            NoSignatures(kind) => write!(
                f,
                "SignatureError: signature verification is only implemented for `.tar.gz` and \
                 `.zip` assets, not {} files",
                kind
            ),
            #[cfg(feature = "signatures")]
            Signature(e) => write!(f, "SignatureError: {}", e),
            #[cfg(feature = "signatures")]
            SignatureNonUTF8 => {
                write!(
                    f,
                    "SignatureError: cannot verify signature of a file with a non-UTF-8 name"
                )
            }
            #[cfg(feature = "s3-auth")]
            S3Auth(e) => write!(f, "S3AuthError: {}", e),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match *self {
            Error::Io(ref e) => e,
            Error::Json(ref e) => &**e,
            Error::Transport(ref e) => &**e,
            Error::SemVer(ref e) => &**e,
            #[cfg(feature = "archive-zip")]
            Error::Zip(ref e) => &**e,
            #[cfg(feature = "signatures")]
            Error::Signature(ref e) => &**e,
            #[cfg(feature = "s3-auth")]
            Error::S3Auth(ref e) => &**e,
            _ => return None,
        })
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Error {
        Error::Json(Box::new(e))
    }
}

#[cfg(feature = "reqwest")]
impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Error {
        Error::Transport(Box::new(e))
    }
}

#[cfg(feature = "ureq")]
impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Error {
        Error::Transport(Box::new(e))
    }
}

impl From<semver::Error> for Error {
    fn from(e: semver::Error) -> Error {
        Error::SemVer(Box::new(e))
    }
}

#[cfg(feature = "archive-zip")]
impl From<ZipError> for Error {
    fn from(e: ZipError) -> Error {
        Error::Zip(Box::new(e))
    }
}

#[cfg(feature = "signatures")]
impl From<ZipsignError> for Error {
    fn from(e: ZipsignError) -> Error {
        Error::Signature(Box::new(e))
    }
}

#[cfg(feature = "s3-auth")]
impl From<std::time::SystemTimeError> for Error {
    fn from(e: std::time::SystemTimeError) -> Self {
        Error::S3Auth(Box::new(e))
    }
}

#[cfg(feature = "s3-auth")]
impl From<hmac::digest::InvalidLength> for Error {
    fn from(e: hmac::digest::InvalidLength) -> Self {
        Error::S3Auth(Box::new(e))
    }
}

#[cfg(feature = "s3-auth")]
impl From<url::ParseError> for Error {
    fn from(e: url::ParseError) -> Self {
        Error::S3Auth(Box::new(e))
    }
}

#[cfg(feature = "s3-auth")]
impl From<time::error::ComponentRange> for Error {
    fn from(e: time::error::ComponentRange) -> Self {
        Error::S3Auth(Box::new(e))
    }
}

/// Map an HTTP status code and URL to the appropriate structured error variant.
///
/// 404 -> `Error::NotFound`, 401/403 -> `Error::Unauthorized`, else -> `Error::HttpStatus`.
pub(crate) fn status_to_error(status: u16, url: &str) -> Error {
    match status {
        404 => Error::NotFound {
            url: url.to_string(),
        },
        401 | 403 => Error::Unauthorized {
            status,
            url: url.to_string(),
        },
        _ => Error::HttpStatus {
            status,
            url: url.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::Error;
    use std::error::Error as _;

    /// Produce a real `serde_json::Error` by parsing malformed JSON.
    fn json_error() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("{").unwrap_err()
    }

    /// Produce a real `semver::Error` by parsing an invalid requirement.
    fn semver_error() -> semver::Error {
        "not a version".parse::<semver::Version>().unwrap_err()
    }

    // C1: `Error::Json` is opaque (boxed). The `From<serde_json::Error>` conversion must produce an
    // `Error::Json` whose `source()` surfaces the underlying boxed error, mirroring `Transport`/`S3Auth`.
    // Pre-fix this variant held a concrete `serde_json::Error` (still `source()`-able, but not
    // boxed); after the box the `source()` arm must deref the box (`&**e`).
    #[test]
    fn json_error_is_opaque_with_source() {
        let err: Error = json_error().into();
        assert!(
            matches!(err, Error::Json(_)),
            "From<serde_json::Error> -> Error::Json"
        );
        assert!(
            err.source().is_some(),
            "Error::Json must expose its underlying error via source()"
        );
    }

    // C1: the boxed `Error::Json` must still render with the `JsonError:` Display prefix and embed
    // the inner error's message (the Display arm dereferences the box, not the box debug form).
    #[test]
    fn json_error_display_includes_prefix_and_inner_message() {
        let inner = json_error();
        let inner_shown = inner.to_string();
        let err: Error = inner.into();
        let shown = err.to_string();
        assert!(
            shown.starts_with("JsonError: "),
            "Error::Json Display must keep the `JsonError: ` prefix, got: {}",
            shown
        );
        assert!(
            shown.contains(&inner_shown),
            "Error::Json Display must embed the inner error message `{}`, got: {}",
            inner_shown,
            shown
        );
    }

    // C1: `Error::SemVer` is opaque (boxed) and surfaces its source via the dereferenced box.
    #[test]
    fn semver_error_is_opaque_with_source() {
        let err: Error = semver_error().into();
        assert!(
            matches!(err, Error::SemVer(_)),
            "From<semver::Error> -> Error::SemVer"
        );
        assert!(
            err.source().is_some(),
            "Error::SemVer must expose its underlying error via source()"
        );
    }

    // C1: the boxed `Error::SemVer` keeps the `SemVerError:` Display prefix and inner message.
    #[test]
    fn semver_error_display_includes_prefix_and_inner_message() {
        let inner = semver_error();
        let inner_shown = inner.to_string();
        let err: Error = inner.into();
        let shown = err.to_string();
        assert!(
            shown.starts_with("SemVerError: "),
            "Error::SemVer Display must keep the `SemVerError: ` prefix, got: {}",
            shown
        );
        assert!(
            shown.contains(&inner_shown),
            "Error::SemVer Display must embed the inner error message `{}`, got: {}",
            inner_shown,
            shown
        );
    }

    // B2: `Error::Zip` is opaque (boxed). The `From<ZipError>` conversion must produce an
    // `Error::Zip` whose `source()` surfaces the underlying boxed error, mirroring `Transport`/`S3Auth`.
    // Pre-fix this variant held a concrete `zip::result::ZipError` and exposed no `source()`.
    #[cfg(feature = "archive-zip")]
    #[test]
    fn zip_error_is_opaque_with_source() {
        let zip_err = zip::result::ZipError::FileNotFound;
        let err: Error = zip_err.into();
        assert!(matches!(err, Error::Zip(_)), "From<ZipError> -> Error::Zip");
        assert!(
            err.source().is_some(),
            "Error::Zip must expose its underlying error via source()"
        );
    }

    // B2: the boxed `Error::Zip` must still render with the `ZipError:` Display prefix and embed
    // the inner error's message. Only `source()` was asserted before the box; this pins that the
    // Display arm dereferences the box rather than printing the box's debug form or being dropped.
    #[cfg(feature = "archive-zip")]
    #[test]
    fn zip_error_display_includes_prefix_and_inner_message() {
        let err: Error = zip::result::ZipError::FileNotFound.into();
        let shown = err.to_string();
        assert!(
            shown.starts_with("ZipError: "),
            "Error::Zip Display must keep the `ZipError: ` prefix, got: {}",
            shown
        );
        // The inner boxed error's own Display must be embedded (not the box debug form).
        let inner = zip::result::ZipError::FileNotFound.to_string();
        assert!(
            shown.contains(&inner),
            "Error::Zip Display must embed the inner error message `{}`, got: {}",
            inner,
            shown
        );
    }

    // B2: `Error::Signature` is opaque (boxed) and surfaces its source. Pre-fix it held a concrete
    // `zipsign_api::ZipsignError`; the `source()` arm now dereferences the box.
    #[cfg(feature = "signatures")]
    #[test]
    fn signature_error_is_opaque_with_source() {
        let inner = zipsign_api::ZipsignError::from(std::io::Error::other("boom"));
        let err: Error = inner.into();
        assert!(
            matches!(err, Error::Signature(_)),
            "From<ZipsignError> -> Error::Signature"
        );
        assert!(
            err.source().is_some(),
            "Error::Signature must expose its underlying error via source()"
        );
    }

    // B2: the boxed `Error::Signature` must still render with the `SignatureError:` Display prefix
    // and embed the inner error's message. Pins the Display arm dereferences the box.
    #[cfg(feature = "signatures")]
    #[test]
    fn signature_error_display_includes_prefix_and_inner_message() {
        let inner = zipsign_api::ZipsignError::from(std::io::Error::other("boom"));
        let inner_shown = inner.to_string();
        let err: Error = inner.into();
        let shown = err.to_string();
        assert!(
            shown.starts_with("SignatureError: "),
            "Error::Signature Display must keep the `SignatureError: ` prefix, got: {}",
            shown
        );
        assert!(
            shown.contains(&inner_shown),
            "Error::Signature Display must embed the inner error message `{}`, got: {}",
            inner_shown,
            shown
        );
    }

    // B7c: the signatures-gated non-UTF8 variant is named `SignatureNonUTF8` (was `NonUTF8`).
    // Naming + Display are pinned here; the rename is what makes this compile.
    // Display prefix corrected to "SignatureError: ..." for consistency with all other variants.
    #[cfg(feature = "signatures")]
    #[test]
    fn signature_non_utf8_variant_is_renamed_and_displays() {
        let err = Error::SignatureNonUTF8;
        assert_eq!(
            err.to_string(),
            "SignatureError: cannot verify signature of a file with a non-UTF-8 name"
        );
    }

    // Transport variant: opaque (boxed), source() derefs the box, Display prefix "TransportError:".
    // From<reqwest::Error> maps to Transport (reqwest feature).
    #[cfg(feature = "reqwest")]
    #[test]
    fn reqwest_error_maps_to_transport_variant() {
        // Construct a reqwest::Error by attempting to parse an invalid URL.
        let e = reqwest::blocking::get("not-a-url").unwrap_err();
        let err: Error = e.into();
        assert!(
            matches!(err, Error::Transport(_)),
            "From<reqwest::Error> must produce Error::Transport, got {:?}",
            err
        );
        assert!(
            err.source().is_some(),
            "Error::Transport must expose its underlying error via source()"
        );
        let shown = err.to_string();
        assert!(
            shown.starts_with("TransportError: "),
            "Error::Transport Display must have 'TransportError: ' prefix, got: {}",
            shown
        );
    }

    // From<ureq::Error> maps to Transport (ureq feature).
    #[cfg(feature = "ureq")]
    #[test]
    fn ureq_error_maps_to_transport_variant() {
        let e = ureq::Error::BadUri("not-a-url".to_string());
        let err: Error = e.into();
        assert!(
            matches!(err, Error::Transport(_)),
            "From<ureq::Error> must produce Error::Transport, got {:?}",
            err
        );
        assert!(
            err.source().is_some(),
            "Error::Transport must expose its underlying error via source()"
        );
        let shown = err.to_string();
        assert!(
            shown.starts_with("TransportError: "),
            "Error::Transport Display must have 'TransportError: ' prefix, got: {}",
            shown
        );
    }

    // NotFound variant Display: "NotFoundError: no resource found at {url} (HTTP 404)"
    #[test]
    fn not_found_display_matches_spec() {
        let err = Error::NotFound {
            url: "https://example.com/releases".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "NotFoundError: no resource found at https://example.com/releases (HTTP 404)"
        );
    }

    // Unauthorized variant Display: "UnauthorizedError: request to {url} was not authorized (HTTP {status})"
    #[test]
    fn unauthorized_display_matches_spec_401() {
        let err = Error::Unauthorized {
            status: 401,
            url: "https://example.com/api".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "UnauthorizedError: request to https://example.com/api was not authorized (HTTP 401)"
        );
    }

    #[test]
    fn unauthorized_display_matches_spec_403() {
        let err = Error::Unauthorized {
            status: 403,
            url: "https://example.com/private".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "UnauthorizedError: request to https://example.com/private was not authorized (HTTP 403)"
        );
    }

    // HttpStatus variant Display: "HttpStatusError: request to {url} failed with status {status}"
    #[test]
    fn http_status_display_matches_spec() {
        let err = Error::HttpStatus {
            status: 503,
            url: "https://example.com/releases".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "HttpStatusError: request to https://example.com/releases failed with status 503"
        );
    }

    // http_status() returns Some(404) for NotFound
    #[test]
    fn http_status_helper_not_found() {
        let err = Error::NotFound {
            url: "u".to_string(),
        };
        assert_eq!(err.http_status(), Some(404));
    }

    // http_status() returns Some(status) for Unauthorized
    #[test]
    fn http_status_helper_unauthorized() {
        assert_eq!(
            Error::Unauthorized {
                status: 401,
                url: "u".to_string()
            }
            .http_status(),
            Some(401)
        );
        assert_eq!(
            Error::Unauthorized {
                status: 403,
                url: "u".to_string()
            }
            .http_status(),
            Some(403)
        );
    }

    // http_status() returns Some(status) for HttpStatus
    #[test]
    fn http_status_helper_http_status_variant() {
        assert_eq!(
            Error::HttpStatus {
                status: 503,
                url: "u".to_string()
            }
            .http_status(),
            Some(503)
        );
        assert_eq!(
            Error::HttpStatus {
                status: 500,
                url: "u".to_string()
            }
            .http_status(),
            Some(500)
        );
    }

    // http_status() returns None for non-HTTP variants
    #[test]
    fn http_status_helper_returns_none_for_non_http_variants() {
        assert_eq!(Error::Update("x".into()).http_status(), None);
        assert_eq!(Error::Release("x".into()).http_status(), None);
        assert_eq!(Error::Config("x".into()).http_status(), None);
        assert_eq!(Error::Io(std::io::Error::other("x")).http_status(), None);
        assert_eq!(Error::Json(Box::new(json_error())).http_status(), None);
        assert_eq!(
            Error::Transport(Box::new(std::io::Error::other("x"))).http_status(),
            None
        );
    }

    // status_to_error maps 404 -> NotFound, 401/403 -> Unauthorized, other -> HttpStatus
    #[test]
    fn status_to_error_maps_404_to_not_found() {
        let e = super::status_to_error(404, "https://example.com/r");
        assert!(
            matches!(e, Error::NotFound { ref url } if url == "https://example.com/r"),
            "status 404 must map to Error::NotFound, got {:?}",
            e
        );
    }

    #[test]
    fn status_to_error_maps_401_to_unauthorized() {
        let e = super::status_to_error(401, "https://example.com/r");
        assert!(
            matches!(e, Error::Unauthorized { status: 401, ref url } if url == "https://example.com/r"),
            "status 401 must map to Error::Unauthorized, got {:?}",
            e
        );
    }

    #[test]
    fn status_to_error_maps_403_to_unauthorized() {
        let e = super::status_to_error(403, "https://example.com/r");
        assert!(
            matches!(e, Error::Unauthorized { status: 403, ref url } if url == "https://example.com/r"),
            "status 403 must map to Error::Unauthorized, got {:?}",
            e
        );
    }

    #[test]
    fn status_to_error_maps_500_to_http_status() {
        let e = super::status_to_error(500, "https://example.com/r");
        assert!(
            matches!(e, Error::HttpStatus { status: 500, ref url } if url == "https://example.com/r"),
            "status 500 must map to Error::HttpStatus, got {:?}",
            e
        );
    }

    #[test]
    fn status_to_error_maps_503_to_http_status() {
        let e = super::status_to_error(503, "https://example.com/r");
        assert!(
            matches!(e, Error::HttpStatus { status: 503, .. }),
            "status 503 must map to Error::HttpStatus, got {:?}",
            e
        );
    }

    // A 3xx redirect that a client did NOT auto-follow is not 404/401/403, so it must fall into the
    // `_ =>` arm and classify as `HttpStatus` carrying its exact code -- never `NotFound` or
    // `Unauthorized`. Pins the redirect-status boundary of the catch-all arm.
    #[test]
    fn status_to_error_maps_3xx_to_http_status() {
        let e = super::status_to_error(301, "https://example.com/r");
        assert!(
            matches!(e, Error::HttpStatus { status: 301, ref url } if url == "https://example.com/r"),
            "status 301 must map to Error::HttpStatus(301), got {:?}",
            e
        );

        let e = super::status_to_error(304, "https://example.com/r");
        assert!(
            matches!(e, Error::HttpStatus { status: 304, .. }),
            "status 304 must map to Error::HttpStatus(304), got {:?}",
            e
        );
    }

    // --- New structured variants (ChecksumMismatch, Aborted) ----------------------------------

    // ChecksumMismatch: exact Display string, no http_status(), no url().
    #[test]
    fn checksum_mismatch_display_exact_string() {
        let err = Error::ChecksumMismatch {
            expected: "aabbcc".to_string(),
            computed: "112233".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "ChecksumMismatchError: checksum mismatch (expected aabbcc, computed 112233)"
        );
    }

    #[test]
    fn checksum_mismatch_http_status_is_none() {
        let err = Error::ChecksumMismatch {
            expected: "aa".to_string(),
            computed: "bb".to_string(),
        };
        assert_eq!(err.http_status(), None);
    }

    #[test]
    fn checksum_mismatch_url_is_none() {
        let err = Error::ChecksumMismatch {
            expected: "aa".to_string(),
            computed: "bb".to_string(),
        };
        assert_eq!(err.url(), None);
    }

    // Aborted: exact Display string.
    #[test]
    fn aborted_display_exact_string() {
        assert_eq!(
            Error::Aborted.to_string(),
            "AbortedError: the update was not confirmed"
        );
    }

    #[test]
    fn aborted_http_status_is_none() {
        assert_eq!(Error::Aborted.http_status(), None);
    }

    #[test]
    fn aborted_url_is_none() {
        assert_eq!(Error::Aborted.url(), None);
    }

    // url() returns Some for NotFound / Unauthorized / HttpStatus, None for non-HTTP variants.
    #[test]
    fn url_helper_not_found() {
        let err = Error::NotFound {
            url: "https://example.com/releases".to_string(),
        };
        assert_eq!(err.url(), Some("https://example.com/releases"));
    }

    #[test]
    fn url_helper_unauthorized() {
        let err = Error::Unauthorized {
            status: 401,
            url: "https://example.com/api".to_string(),
        };
        assert_eq!(err.url(), Some("https://example.com/api"));
    }

    #[test]
    fn url_helper_http_status() {
        let err = Error::HttpStatus {
            status: 503,
            url: "https://example.com/releases".to_string(),
        };
        assert_eq!(err.url(), Some("https://example.com/releases"));
    }

    #[test]
    fn url_helper_returns_none_for_non_http_variants() {
        assert_eq!(Error::Update("x".into()).url(), None);
        assert_eq!(Error::Release("x".into()).url(), None);
        assert_eq!(Error::Config("x".into()).url(), None);
        assert_eq!(Error::Aborted.url(), None);
        assert_eq!(
            Error::ChecksumMismatch {
                expected: "a".into(),
                computed: "b".into()
            }
            .url(),
            None
        );
        assert_eq!(Error::Io(std::io::Error::other("x")).url(), None);
    }

    // ArchiveNotEnabled Display prefix corrected to "ArchiveNotEnabledError: ...".
    #[test]
    fn archive_not_enabled_display_has_correct_prefix() {
        let err = Error::ArchiveNotEnabled("zip".to_string());
        let shown = err.to_string();
        assert!(
            shown.starts_with("ArchiveNotEnabledError: "),
            "ArchiveNotEnabled Display must start with 'ArchiveNotEnabledError: ', got: {}",
            shown
        );
        assert!(
            shown.contains("zip"),
            "ArchiveNotEnabled Display must contain the extension, got: {}",
            shown
        );
    }

    // SignatureNonUTF8 Display prefix corrected to "SignatureError: ...".
    #[cfg(feature = "signatures")]
    #[test]
    fn signature_non_utf8_display_has_signature_error_prefix() {
        let err = Error::SignatureNonUTF8;
        let shown = err.to_string();
        assert!(
            shown.starts_with("SignatureError: "),
            "SignatureNonUTF8 Display must start with 'SignatureError: ', got: {}",
            shown
        );
    }
}

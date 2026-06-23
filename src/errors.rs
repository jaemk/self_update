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
    /// An internal invariant of the update pipeline was violated, or an internal task failed.
    ///
    /// This signals a bug or an unexpected condition (the extractor source has no file name, a
    /// required path was not found in an archive, an archive path was not valid UTF-8, or a
    /// blocking task failed to join), not a normal failure mode a caller can act on. When the
    /// failure wraps an underlying error (e.g. a tokio `JoinError`), it is carried as `source`
    /// and surfaced via [`std::error::Error::source`].
    #[non_exhaustive]
    Internal {
        /// Human-readable description of the violated invariant / failed task.
        message: String,
        /// The underlying error, when this wraps one (e.g. a tokio `JoinError`); else `None`.
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
    /// A post-update verification callback (`verify_binary`) rejected the freshly-extracted binary.
    ///
    /// This is a user-controlled rejection: the caller's `verify_binary` closure returned `Err(..)`
    /// (an explicit rejection or a hook IO error), so nothing was installed. `reason` carries the
    /// hook error's message when one was returned (else `None`).
    #[non_exhaustive]
    VerificationRejected {
        /// The reason the verification was rejected — the hook error's message, if any.
        reason: Option<String>,
    },
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
    /// No release (or no release asset matching the requested target) was found.
    ///
    /// This is the clean negative outcome of a release lookup: the remote listing had no release,
    /// no release matched the requested tag/version, or the resolved release had no asset for
    /// `target`. `target` is the requested target triple when the lookup was asset-scoped, else
    /// `None`.
    #[non_exhaustive]
    NoReleaseFound {
        /// The requested target triple, when the lookup failed to find a matching asset; else `None`.
        target: Option<String>,
    },
    /// A release or asset payload from the backend was missing a required field.
    ///
    /// `field` is the name of the absent field (e.g. `"tag_name"`, `"browser_download_url"`).
    #[non_exhaustive]
    MissingAssetField {
        /// The name of the missing field in the release/asset payload.
        field: &'static str,
    },
    /// A backend response could not be parsed.
    ///
    /// Wraps the underlying parse error (e.g. an S3 XML reader error or a regex build failure),
    /// surfaced via [`std::error::Error::source`].
    #[non_exhaustive]
    InvalidResponse {
        /// The underlying parse error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A required builder/configuration field was not set.
    ///
    /// `field` names the missing field (e.g. `"repo_owner"`, `"bin_name"`, `"region"`).
    #[non_exhaustive]
    MissingField {
        /// The name of the missing required field.
        field: &'static str,
    },
    /// An HTTP header supplied to the builder (`request_header` / `header`) was not valid.
    ///
    /// Wraps the underlying header-conversion error, surfaced via [`std::error::Error::source`].
    #[non_exhaustive]
    InvalidHeader {
        /// The underlying header-conversion error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// An auth token could not be encoded as an HTTP `Authorization` header value.
    ///
    /// Wraps the underlying header-conversion error, surfaced via [`std::error::Error::source`].
    #[non_exhaustive]
    InvalidAuthToken {
        /// The underlying header-conversion error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A configuration error that does not fit a more specific variant.
    ///
    /// Currently produced only by the S3 backend when a host cannot be extracted from a signed
    /// URL (`s3-auth`).
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
            Internal { message, .. } => write!(f, "InternalError: {}", message),
            VerificationRejected { reason } => match reason {
                Some(reason) => write!(
                    f,
                    "VerificationRejectedError: post-update verification rejected the new binary: {}",
                    reason
                ),
                None => write!(
                    f,
                    "VerificationRejectedError: post-update verification rejected the new binary"
                ),
            },
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
            NoReleaseFound { target } => match target {
                Some(target) => write!(
                    f,
                    "ReleaseError: no release found with an asset for target `{}`",
                    target
                ),
                None => write!(f, "ReleaseError: no release was found"),
            },
            MissingAssetField { field } => {
                write!(f, "ReleaseError: release/asset payload missing `{}`", field)
            }
            InvalidResponse { source } => write!(f, "ReleaseError: invalid response: {}", source),
            MissingField { field } => write!(f, "ConfigError: `{}` required", field),
            InvalidHeader { source } => write!(f, "ConfigError: invalid HTTP header: {}", source),
            InvalidAuthToken { source } => {
                write!(f, "ConfigError: failed to parse auth token: {}", source)
            }
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
            Error::Internal {
                source: Some(ref e),
                ..
            } => &**e,
            Error::InvalidResponse { ref source } => &**source,
            Error::InvalidHeader { ref source } => &**source,
            Error::InvalidAuthToken { ref source } => &**source,
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

/// A minimal owned error carrying just a message, used as the boxed `source` for the
/// builder header-validation path where the underlying `TryInto` conversion error is not
/// nameable through the generic bound. Lets `Error::InvalidHeader` still expose a non-`None`
/// `source()` that renders the original validation message.
#[derive(Debug)]
pub(crate) struct MessageError(pub(crate) String);

impl std::fmt::Display for MessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MessageError {}

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
    use super::{Error, MessageError};
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
        assert_eq!(
            Error::Internal {
                message: "x".into(),
                source: None
            }
            .http_status(),
            None
        );
        assert_eq!(Error::NoReleaseFound { target: None }.http_status(), None);
        assert_eq!(Error::MissingField { field: "x" }.http_status(), None);
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
        assert_eq!(
            Error::Internal {
                message: "x".into(),
                source: None
            }
            .url(),
            None
        );
        assert_eq!(Error::NoReleaseFound { target: None }.url(), None);
        assert_eq!(Error::MissingField { field: "x" }.url(), None);
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

    // --- WS3 structured-variant unit tests ----------------------------------------------------

    // `MissingField` Display: "ConfigError: `<field>` required".
    #[test]
    fn missing_field_display_and_no_source() {
        let err = Error::MissingField {
            field: "current_version",
        };
        assert_eq!(err.to_string(), "ConfigError: `current_version` required");
        assert!(
            err.source().is_none(),
            "MissingField carries no source, got {:?}",
            err.source()
        );
        assert_eq!(err.http_status(), None);
        assert_eq!(err.url(), None);
    }

    // `NoReleaseFound` Display differs with/without a target, and never has a source.
    #[test]
    fn no_release_found_display_variants() {
        assert_eq!(
            Error::NoReleaseFound { target: None }.to_string(),
            "ReleaseError: no release was found"
        );
        assert_eq!(
            Error::NoReleaseFound {
                target: Some("x86_64-unknown-linux-gnu".into())
            }
            .to_string(),
            "ReleaseError: no release found with an asset for target `x86_64-unknown-linux-gnu`"
        );
        assert!(Error::NoReleaseFound { target: None }.source().is_none());
    }

    // `MissingAssetField` Display names the absent payload field.
    #[test]
    fn missing_asset_field_display() {
        let err = Error::MissingAssetField { field: "tag_name" };
        assert_eq!(
            err.to_string(),
            "ReleaseError: release/asset payload missing `tag_name`"
        );
        assert!(err.source().is_none());
    }

    // `VerificationRejected` Display, with and without a reason.
    #[test]
    fn verification_rejected_display_variants() {
        assert_eq!(
            Error::VerificationRejected { reason: None }.to_string(),
            "VerificationRejectedError: post-update verification rejected the new binary"
        );
        assert_eq!(
            Error::VerificationRejected {
                reason: Some("bad signature".into())
            }
            .to_string(),
            "VerificationRejectedError: post-update verification rejected the new binary: bad signature"
        );
        assert_eq!(
            Error::VerificationRejected { reason: None }.http_status(),
            None
        );
        assert!(
            Error::VerificationRejected { reason: None }
                .source()
                .is_none()
        );
    }

    // E6: `InvalidResponse` carries a boxed source and chains it through `source()`.
    #[test]
    fn invalid_response_chains_source() {
        let inner = json_error();
        let inner_shown = inner.to_string();
        let err = Error::InvalidResponse {
            source: Box::new(inner),
        };
        let chained = err
            .source()
            .expect("InvalidResponse must expose its source()");
        assert!(
            chained.to_string().contains(&inner_shown),
            "source() must surface the inner error, got: {}",
            chained
        );
        assert!(
            err.to_string()
                .starts_with("ReleaseError: invalid response: ")
        );
    }

    // E1/E6: `InvalidHeader` carries a boxed source and chains it through `source()`.
    #[test]
    fn invalid_header_chains_source() {
        let err = Error::InvalidHeader {
            source: Box::new(MessageError("bad header".into())),
        };
        assert_eq!(
            err.source().map(|s| s.to_string()).as_deref(),
            Some("bad header")
        );
        assert!(
            err.to_string()
                .starts_with("ConfigError: invalid HTTP header: ")
        );
    }

    // E1/E6: `InvalidAuthToken` carries a boxed source and chains it through `source()`.
    #[test]
    fn invalid_auth_token_chains_source() {
        // A control char produces a real header-value parse error.
        let inner = "bad\nvalue".parse::<crate::http_client::header::HeaderValue>();
        let inner = inner.expect_err("control char must fail header parse");
        let inner_shown = inner.to_string();
        let err = Error::InvalidAuthToken {
            source: Box::new(inner),
        };
        let chained = err
            .source()
            .expect("InvalidAuthToken must expose its source()");
        assert!(chained.to_string().contains(&inner_shown));
        assert!(
            err.to_string()
                .starts_with("ConfigError: failed to parse auth token: ")
        );
    }

    // E3/E6: `Internal` with a source chains it; without a source returns None.
    #[test]
    fn internal_source_chaining() {
        let with = Error::Internal {
            message: "boom".into(),
            source: Some(Box::new(MessageError("inner".into()))),
        };
        assert_eq!(with.to_string(), "InternalError: boom");
        assert_eq!(
            with.source().map(|s| s.to_string()).as_deref(),
            Some("inner")
        );

        let without = Error::Internal {
            message: "boom".into(),
            source: None,
        };
        assert!(
            without.source().is_none(),
            "Internal without a source must return None"
        );
    }

    // E4: `Io` still carries the concrete `std::io::Error` (not boxed), exposing `ErrorKind`.
    #[test]
    fn io_error_exposes_error_kind() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "nope",
        ));
        match err {
            Error::Io(ref io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected Error::Io, got {:?}", other),
        }
    }

    // E4: `Error` is `#[non_exhaustive]`, so a downstream-style `match` with a wildcard arm
    // compiles and the new struct variants stay non-breaking to add to.
    #[test]
    fn non_exhaustive_match_with_wildcard_compiles() {
        fn classify(err: &Error) -> &'static str {
            match err {
                Error::MissingField { .. } => "missing-field",
                Error::NoReleaseFound { .. } => "no-release",
                Error::VerificationRejected { .. } => "verify-rejected",
                Error::Internal { .. } => "internal",
                // The mandatory wildcard arm: required because `Error` is `#[non_exhaustive]`.
                _ => "other",
            }
        }
        assert_eq!(
            classify(&Error::MissingField { field: "x" }),
            "missing-field"
        );
        assert_eq!(
            classify(&Error::NoReleaseFound { target: None }),
            "no-release"
        );
        assert_eq!(classify(&Error::Aborted), "other");
    }

    // E4/WS3: the `#[non_exhaustive]` struct variants require a trailing `..` to destructure from a
    // downstream perspective (adding a field stays non-breaking). A destructure that binds the
    // current fields plus `..` must compile and read them. This pins the struct-level
    // non_exhaustive contract that the enum-level wildcard test above does not exercise.
    #[test]
    fn non_exhaustive_struct_variants_destructure_with_rest() {
        // `Internal` carries `message` + `source`; bind `message`, ignore the rest via `..`.
        let internal = Error::Internal {
            message: "boom".into(),
            source: None,
        };
        if let Error::Internal { message, .. } = &internal {
            assert_eq!(message, "boom");
        } else {
            panic!("expected Internal");
        }

        // `NoReleaseFound` carries `target`; bind it with `..` for forward-compatibility.
        let nrf = Error::NoReleaseFound {
            target: Some("t".into()),
        };
        if let Error::NoReleaseFound { target, .. } = &nrf {
            assert_eq!(target.as_deref(), Some("t"));
        } else {
            panic!("expected NoReleaseFound");
        }

        // `Unauthorized` is `#[non_exhaustive]` too; `..` lets us read just `status`.
        let unauth = Error::Unauthorized {
            status: 401,
            url: "u".into(),
        };
        if let Error::Unauthorized { status, .. } = &unauth {
            assert_eq!(*status, 401);
        } else {
            panic!("expected Unauthorized");
        }
    }

    // WS3 Display: every restructured/new variant has a non-panicking Display that keeps a sensible
    // prefix and embeds its data. The per-variant tests above cover the exact strings; this is a
    // belt-and-suspenders sweep that no new variant lost its message or panics on Display.
    #[test]
    fn all_new_variants_display_without_panicking() {
        let cases: Vec<(Error, &str)> = vec![
            (
                Error::Internal {
                    message: "m".into(),
                    source: None,
                },
                "InternalError:",
            ),
            (
                Error::VerificationRejected { reason: None },
                "VerificationRejectedError:",
            ),
            (Error::NoReleaseFound { target: None }, "ReleaseError:"),
            (Error::MissingAssetField { field: "f" }, "ReleaseError:"),
            (
                Error::InvalidResponse {
                    source: Box::new(MessageError("x".into())),
                },
                "ReleaseError:",
            ),
            (Error::MissingField { field: "f" }, "ConfigError:"),
            (
                Error::InvalidHeader {
                    source: Box::new(MessageError("x".into())),
                },
                "ConfigError:",
            ),
            (
                Error::InvalidAuthToken {
                    source: Box::new(MessageError("x".into())),
                },
                "ConfigError:",
            ),
        ];
        for (err, prefix) in cases {
            let shown = err.to_string();
            assert!(
                shown.starts_with(prefix),
                "{:?} Display must start with `{}`, got: {}",
                err,
                prefix,
                shown
            );
            assert!(!shown.is_empty(), "Display must not be empty");
        }
    }
}

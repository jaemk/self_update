/*!
Error type, conversions, and macros

*/
#[cfg(feature = "archive-zip")]
use zip::result::ZipError;

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(feature = "signatures")]
use zipsign_api::ZipsignError;

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Used as a catch-most for when the program fails to update.
    Update(String),
    /// Used when a web request to a repository archive fails.
    Network(String),
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
    /// A wrapper over the active HTTP client's error type (`reqwest` or `ureq`).
    ///
    /// The concrete error is boxed so that the public API does not change when the
    /// `reqwest` / `ureq` feature selection changes. Use [`std::error::Error::source`]
    /// to inspect the underlying error.
    Http(Box<dyn std::error::Error + Send + Sync>),
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

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use Error::*;
        match *self {
            Update(ref s) => write!(f, "UpdateError: {}", s),
            Network(ref s) => write!(f, "NetworkError: {}", s),
            Release(ref s) => write!(f, "ReleaseError: {}", s),
            Config(ref s) => write!(f, "ConfigError: {}", s),
            Io(ref e) => write!(f, "IoError: {}", e),
            Json(ref e) => write!(f, "JsonError: {}", e),
            Http(ref e) => write!(f, "HttpError: {}", e),
            SemVer(ref e) => write!(f, "SemVerError: {}", e),
            #[cfg(feature = "archive-zip")]
            Zip(ref e) => write!(f, "ZipError: {}", e),
            ArchiveNotEnabled(ref s) => write!(f, "ArchiveNotEnabled: Archive extension '{}' not supported, please enable 'archive-{}' feature!", s, s),
            #[cfg(feature = "signatures")]
            NoSignatures(kind) => {
                write!(f, "No signature verification implemented for {:?} files", kind)
            }
            #[cfg(feature = "signatures")]
            Signature(ref e) => write!(f, "SignatureError: {}", e),
            #[cfg(feature = "signatures")]
            SignatureNonUTF8 => {
                write!(f, "Cannot verify signature of a file with a non-UTF-8 name")
            }
            #[cfg(feature = "s3-auth")]
            S3Auth(ref e) => write!(f, "S3AuthError: {}", e),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match *self {
            Error::Io(ref e) => e,
            Error::Json(ref e) => &**e,
            Error::Http(ref e) => &**e,
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
        Error::Http(Box::new(e))
    }
}

#[cfg(feature = "ureq")]
impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Error {
        Error::Http(Box::new(e))
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
    // `Error::Json` whose `source()` surfaces the underlying boxed error, mirroring `Http`/`S3Auth`.
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
    // `Error::Zip` whose `source()` surfaces the underlying boxed error, mirroring `Http`/`S3Auth`.
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
    #[cfg(feature = "signatures")]
    #[test]
    fn signature_non_utf8_variant_is_renamed_and_displays() {
        let err = Error::SignatureNonUTF8;
        assert_eq!(
            err.to_string(),
            "Cannot verify signature of a file with a non-UTF-8 name"
        );
    }
}

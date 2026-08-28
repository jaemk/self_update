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
    /// A pre-extraction verification callback (`verify_archive`) rejected the downloaded archive.
    ///
    /// This is a user-controlled rejection: the caller's `verify_archive` closure returned `Err(..)`
    /// (an explicit rejection or a hook IO error), so nothing was extracted and nothing was
    /// installed. `reason` carries the hook error's message when one was returned (else `None`).
    ///
    /// Distinct from [`VerificationRejected`](Error::VerificationRejected), which is the
    /// `verify_binary` hook's rejection of the *extracted binary*: the two hooks see different
    /// files, so they report through different variants.
    #[non_exhaustive]
    ArchiveVerificationRejected {
        /// The reason the verification was rejected — the hook error's message, if any.
        reason: Option<String>,
    },
    /// A checksum could not be resolved from the release's published sums asset
    /// (`checksum_from_asset`).
    ///
    /// Raised before the artifact is verified, not on a mismatch: the release carries no asset with
    /// the configured name, the sums file is not UTF-8 text, it has no entry for the selected
    /// asset, or the entry's digest is not a supported length. `asset` is the artifact a digest was
    /// being resolved for; `reason` says which of those it was. A digest that *is* resolved and
    /// then does not match produces [`ChecksumMismatch`](Error::ChecksumMismatch) as usual.
    #[non_exhaustive]
    ChecksumSourceInvalid {
        /// The release asset a checksum was being resolved for.
        asset: String,
        /// Why no checksum could be resolved.
        reason: String,
    },
    /// The downloaded artifact's checksum did not match the expected digest.
    ///
    /// `expected` is the configured digest; `computed` is the one actually produced from the
    /// downloaded file. Both are hex-encoded lowercase digests.
    #[non_exhaustive]
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
    #[non_exhaustive]
    NotFound {
        /// The URL whose response was HTTP 404.
        url: String,
    },
    /// A request completed and returned HTTP 401 or 403 (not authorized).
    ///
    /// `status` is the exact HTTP status code (401 or 403). `url` is the request URL.
    ///
    /// Not every 403 lands here: a 403 whose headers report a spent quota or ask for a
    /// `Retry-After` is classified as [`RateLimited`](Error::RateLimited) instead, so code that
    /// matched `Unauthorized { status: 403, .. }` to detect rate limiting must match that variant.
    #[non_exhaustive]
    Unauthorized {
        /// The HTTP status code (401 or 403).
        status: u16,
        /// The URL whose response was this status.
        url: String,
    },
    /// A request was rejected because the caller's request quota is exhausted.
    ///
    /// Distinguished from [`Unauthorized`](Error::Unauthorized) so a caller can tell "wait for the
    /// window to reset, or set a token" from "these credentials are wrong": the forges answer a
    /// rate-limited request with the same 403 they use for a bad credential, and only the
    /// quota headers separate the two. A response is classified here when it is a 429 (RFC 6585
    /// defines that status as rate limiting), or a 403 carrying a zero remaining-quota header
    /// (`x-ratelimit-remaining: 0`, or gitlab's `RateLimit-Remaining: 0`) or a usable
    /// `Retry-After` (GitHub's *secondary* rate limit answers 403 + `Retry-After` while
    /// `x-ratelimit-remaining` is still nonzero).
    ///
    /// The most common cause is the unauthenticated GitHub budget of 60 requests/hour, which is
    /// counted **per source IP** and so is pooled across everyone behind a shared egress IP (a
    /// NAT'd corporate network). Setting a token moves the count to the token's own 5000/hour
    /// budget; see `auth_token_from_env()` on the backend builders.
    ///
    /// Retrying immediately only consumes more quota — back off by
    /// [`rate_limit_delay()`](Error::rate_limit_delay), or check less often (see
    /// [`UpdateCheckGuard`](crate::check_interval::UpdateCheckGuard)).
    ///
    /// **This classification is derived from untrusted response headers.** A server (or anything
    /// on the path able to shape the response) chooses the status and the quota headers, so
    /// receiving `RateLimited` is *not* proof that the credential in use is valid: a hostile or
    /// misconfigured endpoint can present a genuine authorization failure as a rate limit. Treat
    /// it as a hint for back-off, never as an authentication result.
    ///
    /// This variant is `#[non_exhaustive]`, so it cannot be built with a struct literal from
    /// outside this crate. A downstream consumer that needs one — for example to exercise their
    /// own back-off handler against a synthetic response — constructs it through
    /// [`Error::http_status_error_with_headers`], which is the only public entry point that
    /// produces this variant.
    #[non_exhaustive]
    RateLimited {
        /// The HTTP status code (403 or 429).
        status: u16,
        /// The URL whose response was this status.
        url: String,
        /// When the quota window resets, from the response's `x-ratelimit-reset` /
        /// `RateLimit-Reset` header (a unix timestamp); `None` when absent or unparseable.
        ///
        /// This is a **server-supplied absolute instant** that is only meaningful when compared
        /// against the *local* clock, so client clock skew flows straight into any wait derived
        /// from it (a client running behind the server sees a longer wait, one running ahead sees
        /// a shorter one or none at all). Values more than 24h in the future are rejected as
        /// `None`; see [`rate_limit_delay()`](Error::rate_limit_delay).
        reset_at: Option<std::time::SystemTime>,
        /// The delay requested by the response's `Retry-After` header; `None` when absent. Only
        /// the delta-seconds form is parsed — the HTTP-date form yields `None`.
        ///
        /// Server-supplied, and therefore capped: a delay above 24h is rejected as `None` rather
        /// than carried through, so a hostile `Retry-After` cannot park a caller (and with it a
        /// security-update channel) indefinitely. A delay of exactly zero is also `None`: it is
        /// treated as no signal at all rather than "wait zero seconds", so that a bare 403 whose
        /// only rate-limit header is a literal `Retry-After: 0` stays
        /// [`Unauthorized`](Error::Unauthorized) instead of a `RateLimited` with a zero-second wait.
        retry_after: Option<std::time::Duration>,
    },
    /// A request completed and returned a non-2xx status other than 404, 401, or 403.
    ///
    /// `status` is the HTTP status code. `url` is the request URL.
    ///
    /// 429 never lands here: it is always [`RateLimited`](Error::RateLimited), with or without
    /// quota headers, from either constructor
    /// ([`http_status_error`](Error::http_status_error) and
    /// [`http_status_error_with_headers`](Error::http_status_error_with_headers)).
    #[non_exhaustive]
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
    /// `field` is the name of the absent field (e.g. `"tag_name"`, `"browser_download_url"`),
    /// or a path to it (e.g. `"assets[2].url"`).
    #[non_exhaustive]
    MissingAssetField {
        /// The name of (or path to) the missing field in the release/asset payload.
        field: String,
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
    /// The install path (or the directory it lives in) is not writable by this process.
    ///
    /// Returned either by the opt-in preflight writability check
    /// (`check_install_path_writable(true)`), which probes before any download, or by the install
    /// step itself when the replace/move fails with a permission error. `path` is the path that
    /// could not be written: the configured `bin_install_path`, or in bundle mode the
    /// `bundle_install_path` (the preflight names its parent directory, which is where the swap
    /// needs permission). Re-run with elevated privileges, or configure a user-writable install
    /// path; the crate never escalates privileges itself.
    #[non_exhaustive]
    InstallPathNotWritable {
        /// The path that could not be written.
        path: std::path::PathBuf,
    },
    /// Bundle mode is on but the bundle install path could not be derived: the running executable
    /// (`current_exe()`) has no `.app` ancestor.
    ///
    /// Only produced on macOS, where `bundle_install_path` defaults to the nearest enclosing
    /// `.app` directory. Set `bundle_install_path` explicitly (on every other platform it is
    /// required in bundle mode, surfacing as
    /// [`MissingField`](Error::MissingField) instead).
    #[non_exhaustive]
    NoAppBundle {
        /// The running executable that has no enclosing `.app` bundle.
        exe: std::path::PathBuf,
    },
    /// Two builder settings that cannot both apply were set.
    ///
    /// `field` is the setting that was rejected and `conflict` the one it clashes with, e.g.
    /// `bundle_path_in_archive` (which replaces a whole directory bundle) together with an
    /// explicit `bin_install_path` (which replaces a single file). Returned from `build()`, before
    /// any request is made.
    #[non_exhaustive]
    ConflictingConfig {
        /// The setting that was rejected.
        field: &'static str,
        /// The already-set setting it conflicts with.
        conflict: &'static str,
    },
    /// The running app is a translocated copy, so its bundle cannot be replaced.
    ///
    /// macOS runs a quarantined (freshly downloaded, un-cleared) app from a read-only randomized
    /// `AppTranslocation` mount, so the enclosing `.app` path is not the installed one and is not
    /// writable. Move the app (for example to `/Applications`), which clears the quarantine flag,
    /// and relaunch it before updating.
    #[non_exhaustive]
    AppTranslocated {
        /// The running executable, inside the translocated bundle.
        exe: std::path::PathBuf,
    },
    /// A bare release listing ([`ReleaseList::fetch`](crate::backends)) carries no current version,
    /// so [`Releases::is_update_available`](crate::update::Releases::is_update_available) has nothing
    /// to compare its releases against.
    ///
    /// Distinct from [`MissingField`](Error::MissingField): there is no builder field to set. Use
    /// `Update::is_update_available` on a configured updater (which knows its current version)
    /// instead.
    NoCurrentVersion,
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
    /// A custom TLS root certificate could not be parsed, or the HTTP client that would trust it
    /// could not be built.
    ///
    /// Produced from `build()` (via a backend builder's `add_root_certificate`) or from a
    /// [`Download`](crate::Download) with a `root_certificate`. Wraps the underlying error, surfaced
    /// via [`std::error::Error::source`].
    #[non_exhaustive]
    InvalidCertificate {
        /// The underlying certificate-parse / client-build error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A progress-bar template string was not valid (`progress-bar`).
    ///
    /// Wraps the underlying `indicatif` template error, surfaced via [`std::error::Error::source`].
    #[cfg(feature = "progress-bar")]
    #[non_exhaustive]
    InvalidProgressStyle {
        /// The underlying template-parse error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
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
    /// Used when the archive container feature (`archive-tar` / `archive-zip`) for the detected
    /// asset is not enabled. The string is the archive token (`"tar"` / `"zip"`).
    ArchiveNotEnabled(String),
    /// The asset is compressed with a codec whose feature is not enabled.
    ///
    /// The string is the codec token (`"gz"`). Enable the matching feature (`compression-tar-gz`
    /// for gzip) to decode it. Distinct from [`ArchiveNotEnabled`](Error::ArchiveNotEnabled), which
    /// concerns the container format; without this, a gzip asset would install its still-compressed
    /// bytes as the binary.
    CompressionNotEnabled(String),
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
    /// The release asset name contains path traversal components or separators.
    ///
    /// Returned when the server-supplied asset name is empty, is `.` or `..`, contains a `/` or
    /// `\` path separator, or is an absolute path. The file would never be created in that case,
    /// so callers do not need to clean up temporary state.
    #[non_exhaustive]
    InvalidAssetName {
        /// The offending asset name as received from the release listing.
        name: String,
    },
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
    /// A user-supplied `asset_key_pattern` on the s3 builders was not a valid regex, or was
    /// missing a required named capture group (`name` / `version`).
    ///
    /// Returned from `build()`. Wraps the underlying regex-compile error (or a message naming
    /// the missing group), surfaced via [`std::error::Error::source`].
    #[cfg(feature = "s3")]
    #[non_exhaustive]
    InvalidAssetKeyPattern {
        /// The underlying regex-compile error, or a message naming the missing capture group.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl Error {
    /// The HTTP status code if this error came from a completed non-2xx response
    /// (`NotFound` => 404, `Unauthorized`/`RateLimited`/`HttpStatus` => their code); `None`
    /// otherwise.
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Error::NotFound { .. } => Some(404),
            Error::Unauthorized { status, .. } => Some(*status),
            Error::RateLimited { status, .. } => Some(*status),
            Error::HttpStatus { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The URL of the request that failed, for the HTTP error variants
    /// (`NotFound`/`Unauthorized`/`RateLimited`/`HttpStatus`); `None` otherwise.
    pub fn url(&self) -> Option<&str> {
        match self {
            Error::NotFound { url } => Some(url.as_str()),
            Error::Unauthorized { url, .. } => Some(url.as_str()),
            Error::RateLimited { url, .. } => Some(url.as_str()),
            Error::HttpStatus { url, .. } => Some(url.as_str()),
            _ => None,
        }
    }

    /// How long to wait before retrying a [`RateLimited`](Error::RateLimited) request, measured
    /// from *now*; `None` for every other variant.
    ///
    /// Prefers the server's explicit `Retry-After` delay and otherwise derives one from
    /// `reset_at` minus the current time. `None` means "no wait is known, or the window has
    /// already elapsed" — i.e. retry when you like, not "retry immediately is safe forever".
    ///
    /// Use this instead of reading the fields directly: neither one alone is the answer. On
    /// GitHub's *primary* rate limit only `x-ratelimit-reset` is sent, so
    /// `retry_after.unwrap_or_default()` sleeps zero and burns more quota; conversely
    /// `reset_at.unwrap().duration_since(SystemTime::now()).unwrap()` panics once the window has
    /// passed. Both server-supplied values are capped at 24h (see the field docs), and `reset_at`
    /// is compared against the local clock, so client skew shifts the result.
    ///
    /// ```rust
    /// # use self_update::Error;
    /// # use std::time::Duration;
    /// # let mut headers = self_update::http_client::HeaderMap::new();
    /// # headers.insert("retry-after", "60".parse().unwrap());
    /// let err = Error::http_status_error_with_headers(403, "https://api.example.com/x", &headers);
    /// assert_eq!(err.rate_limit_delay(), Some(Duration::from_secs(60)));
    /// ```
    pub fn rate_limit_delay(&self) -> Option<std::time::Duration> {
        match self {
            Error::RateLimited {
                reset_at,
                retry_after,
                ..
            } => retry_after.or_else(|| {
                reset_at.and_then(|at| at.duration_since(std::time::SystemTime::now()).ok())
            }),
            _ => None,
        }
    }

    // --- constructors for custom `ReleaseSource` implementors --------------------------------
    //
    // The release-flow variants are `#[non_exhaustive]`, so downstream code cannot build them with
    // a struct literal. These constructors let a custom source return the canonical error for a
    // condition (no release, a malformed response, a bad status) instead of an opaque catch-all.

    /// Construct a [`NoReleaseFound`](Error::NoReleaseFound) error: the listing had no release, or
    /// no release matched the requested tag/version. For a lookup that failed to find an asset for
    /// a specific target triple, use
    /// [`no_release_found_for_target`](Error::no_release_found_for_target).
    pub fn no_release_found() -> Error {
        Error::NoReleaseFound { target: None }
    }

    /// Construct a [`NoReleaseFound`](Error::NoReleaseFound) error for an asset-scoped lookup:
    /// a release was resolved but had no asset matching `target`.
    pub fn no_release_found_for_target(target: impl Into<String>) -> Error {
        Error::NoReleaseFound {
            target: Some(target.into()),
        }
    }

    /// Construct a [`MissingAssetField`](Error::MissingAssetField) error for a release/asset payload
    /// missing a required field. `field` names the absent field, or a path to it
    /// (e.g. `format!("assets[{i}].url")`).
    pub fn missing_asset_field(field: impl Into<String>) -> Error {
        Error::MissingAssetField {
            field: field.into(),
        }
    }

    /// Construct a [`ChecksumMismatch`](Error::ChecksumMismatch) error from the expected and
    /// computed digests (both hex-encoded lowercase).
    pub fn checksum_mismatch(expected: impl Into<String>, computed: impl Into<String>) -> Error {
        Error::ChecksumMismatch {
            expected: expected.into(),
            computed: computed.into(),
        }
    }

    /// Construct an [`InvalidResponse`](Error::InvalidResponse) error wrapping the underlying parse
    /// error.
    pub fn invalid_response(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
        Error::InvalidResponse {
            source: source.into(),
        }
    }

    /// Construct the HTTP status error for a completed non-2xx response: `NotFound` for 404,
    /// `Unauthorized` for 401/403, [`RateLimited`](Error::RateLimited) for 429, else `HttpStatus`.
    ///
    /// A 429 is a rate limit by RFC 6585 whether or not the response carried quota headers, so it
    /// classifies as `RateLimited` here too -- with `reset_at` and `retry_after` both `None`, since
    /// this form cannot see the headers that supply the wait
    /// ([`rate_limit_delay()`](Error::rate_limit_delay) is then `None`: "no wait is known").
    ///
    /// A custom [`HttpClient`](crate::http_client::HttpClient) that has the response in hand should
    /// call [`http_status_error_with_headers`](Error::http_status_error_with_headers) instead: it
    /// recovers that wait, and it is the only form that can tell a spent-quota 403 (`RateLimited`)
    /// from a credential failure (`Unauthorized`), which this form always reports as the latter.
    pub fn http_status_error(status: u16, url: impl Into<String>) -> Error {
        status_to_error(status, &url.into())
    }

    /// Header-aware [`http_status_error`](Error::http_status_error), for a custom
    /// [`HttpClient`](crate::http_client::HttpClient) / [`AsyncHttpClient`](crate::http_client::AsyncHttpClient)
    /// mapping a non-2xx response: a 429, or a 403 carrying a zero remaining-quota header
    /// (`x-ratelimit-remaining` / `RateLimit-Remaining`) or a usable `Retry-After`, becomes
    /// [`RateLimited`](Error::RateLimited), picking up the reset instant and `Retry-After` delay
    /// when present; every other status classifies exactly as `http_status_error` does.
    ///
    /// ```rust
    /// # use self_update::{Error, http_client::HeaderMap};
    /// let mut headers = HeaderMap::new();
    /// headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
    /// let err = Error::http_status_error_with_headers(403, "https://api.example.com/x", &headers);
    /// assert!(matches!(err, Error::RateLimited { status: 403, .. }));
    /// ```
    pub fn http_status_error_with_headers(
        status: u16,
        url: impl Into<String>,
        headers: &crate::http_client::HeaderMap,
    ) -> Error {
        status_to_error_with_headers(status, &url.into(), headers)
    }

    /// Construct a [`Transport`](Error::Transport) error wrapping the underlying
    /// connection/TLS/timeout failure, for a custom [`HttpClient`](crate::http_client::HttpClient) /
    /// [`AsyncHttpClient`](crate::http_client::AsyncHttpClient) whose request could not be
    /// completed. Accepts an error value or a message string:
    /// `Error::transport(io_err)` / `Error::transport("connection reset by proxy")`.
    pub fn transport(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
        Error::Transport(source.into())
    }

    /// Construct a [`VerificationRejected`](Error::VerificationRejected) error with the given
    /// reason, for rejecting the extracted binary from a `verify_binary` hook:
    ///
    /// ```rust
    /// # fn check(path: &std::path::Path) -> bool { true }
    /// # let hook =
    /// |path: &std::path::Path| {
    ///     if check(path) {
    ///         Ok(())
    ///     } else {
    ///         Err(self_update::Error::verification_rejected("new binary failed --version"))
    ///     }
    /// }
    /// # ;
    /// ```
    ///
    /// The update pipeline surfaces this error as-is; any *other* error returned from the hook is
    /// wrapped in a `VerificationRejected` whose `reason` is that error's message.
    pub fn verification_rejected(reason: impl Into<String>) -> Error {
        Error::VerificationRejected {
            reason: Some(reason.into()),
        }
    }

    /// Construct an [`ArchiveVerificationRejected`](Error::ArchiveVerificationRejected) error with
    /// the given reason, for rejecting the downloaded archive from a `verify_archive` hook:
    ///
    /// ```rust
    /// # fn attested(path: &std::path::Path) -> bool { true }
    /// # let hook =
    /// |archive: &std::path::Path| {
    ///     if attested(archive) {
    ///         Ok(())
    ///     } else {
    ///         Err(self_update::Error::archive_verification_rejected(
    ///             "no build-provenance attestation for this artifact",
    ///         ))
    ///     }
    /// }
    /// # ;
    /// ```
    ///
    /// The update pipeline surfaces this error as-is; any *other* error returned from the hook is
    /// wrapped in an `ArchiveVerificationRejected` whose `reason` is that error's message.
    pub fn archive_verification_rejected(reason: impl Into<String>) -> Error {
        Error::ArchiveVerificationRejected {
            reason: Some(reason.into()),
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
            ArchiveVerificationRejected { reason } => match reason {
                Some(reason) => write!(
                    f,
                    "ArchiveVerificationRejectedError: verification rejected the downloaded archive: {}",
                    reason
                ),
                None => write!(
                    f,
                    "ArchiveVerificationRejectedError: verification rejected the downloaded archive"
                ),
            },
            ChecksumSourceInvalid { asset, reason } => write!(
                f,
                "ChecksumSourceInvalidError: could not resolve a checksum for `{}`: {}",
                asset, reason
            ),
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
            RateLimited {
                status,
                url,
                retry_after,
                ..
            } => {
                write!(
                    f,
                    "RateLimitedError: request to {} was rate limited (HTTP {})",
                    url, status
                )?;
                // `Retry-After` is a requested back-off, not necessarily proof the quota is spent
                // (GitHub's secondary rate limit answers 403 + `Retry-After` while the primary
                // quota is still nonzero), so it gets its own wording rather than "quota resets".
                // Absent that, the wait comes from `reset_at` via `rate_limit_delay` -- that
                // accessor is the one implementation of the precedence, so the rendered countdown
                // and a caller's programmatic back-off can never disagree.
                match retry_after {
                    Some(wait) => write!(f, "; retry in {}s", wait.as_secs())?,
                    None => {
                        if let Some(wait) = self.rate_limit_delay() {
                            write!(f, "; quota resets in {}s", wait.as_secs())?;
                        }
                    }
                }
                write!(
                    f,
                    "; set an auth token to raise the limit, or check less often"
                )
            }
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
            InstallPathNotWritable { path } => write!(
                f,
                "InstallPathNotWritableError: cannot write to install path {}: run with elevated \
                 privileges or configure a user-writable install path",
                path.display()
            ),
            NoAppBundle { exe } => write!(
                f,
                "ConfigError: no `.app` ancestor of {}; set bundle_install_path explicitly",
                exe.display()
            ),
            ConflictingConfig { field, conflict } => write!(
                f,
                "ConfigError: `{}` conflicts with `{}`; set one or the other",
                field, conflict
            ),
            AppTranslocated { exe } => write!(
                f,
                "AppTranslocatedError: {} is running from a translocated (quarantined) copy on a \
                 read-only mount, so its bundle cannot be replaced: move the app (e.g. to \
                 /Applications) and relaunch it before updating",
                exe.display()
            ),
            NoCurrentVersion => write!(
                f,
                "ReleaseError: this Releases has no current_version to compare against; use \
                 `Update::is_update_available` for a configured updater"
            ),
            InvalidHeader { source } => write!(f, "ConfigError: invalid HTTP header: {}", source),
            InvalidAuthToken { source } => {
                write!(f, "ConfigError: failed to parse auth token: {}", source)
            }
            InvalidCertificate { source } => {
                write!(f, "ConfigError: invalid root certificate: {}", source)
            }
            #[cfg(feature = "progress-bar")]
            InvalidProgressStyle { source } => {
                write!(f, "ConfigError: invalid progress bar template: {}", source)
            }
            Io(e) => write!(f, "IoError: {}", e),
            Json(e) => write!(f, "JsonError: {}", e),
            Transport(e) => write!(f, "TransportError: {}", e),
            SemVer(e) => write!(f, "SemVerError: {}", e),
            #[cfg(feature = "archive-zip")]
            Zip(e) => write!(f, "ZipError: {}", e),
            ArchiveNotEnabled(s) => write!(
                f,
                "ArchiveNotEnabledError: archive extension '{}' not supported; enable the 'archive-{}' feature",
                s, s
            ),
            CompressionNotEnabled(s) => write!(
                f,
                "CompressionNotEnabledError: '{}' compression not supported, please enable the 'compression-tar-gz' feature (a `.tar.gz` also needs 'archive-tar')",
                s
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
            InvalidAssetName { name } => {
                write!(f, "InvalidAssetNameError: unsafe asset name: {:?}", name)
            }
            #[cfg(feature = "signatures")]
            SignatureNonUTF8 => {
                write!(
                    f,
                    "SignatureError: cannot verify signature of a file with a non-UTF-8 name"
                )
            }
            #[cfg(feature = "s3-auth")]
            S3Auth(e) => write!(f, "S3AuthError: {}", e),
            #[cfg(feature = "s3")]
            InvalidAssetKeyPattern { source } => {
                write!(f, "ConfigError: invalid asset_key_pattern: {}", source)
            }
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
            Error::InvalidCertificate { ref source } => &**source,
            #[cfg(feature = "progress-bar")]
            Error::InvalidProgressStyle { ref source } => &**source,
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
            #[cfg(feature = "s3")]
            Error::InvalidAssetKeyPattern { ref source } => &**source,
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

/// Map an HTTP status code and URL to the appropriate structured error variant, without seeing the
/// response headers.
///
/// 404 -> `Error::NotFound`, 401/403 -> `Error::Unauthorized`, 429 -> `Error::RateLimited`,
/// else -> `Error::HttpStatus`.
///
/// 429 is classified here, not only in [`classify_status`], because RFC 6585 defines the status
/// itself as rate limiting: the headers only supply the *wait*, never the meaning, so a 429 with no
/// headers in hand is still a rate limit (carrying `reset_at: None` / `retry_after: None`). 401/403
/// deliberately stay [`Error::Unauthorized`] on this path: a 403 genuinely needs a header to tell a
/// spent quota from a bad credential, which is the whole point of the separate variant.
///
/// The URL is stored redacted (see [`redact_url`]) so an s3 presigned request URL does not carry a
/// live `X-Amz-Signature` or the `X-Amz-Credential` access-key id into error messages, logs, or the
/// `url()` accessor.
pub(crate) fn status_to_error(status: u16, url: &str) -> Error {
    let url = redact_url(url);
    match status {
        404 => Error::NotFound { url },
        401 | 403 => Error::Unauthorized { status, url },
        429 => Error::RateLimited {
            status,
            url,
            reset_at: None,
            retry_after: None,
        },
        _ => Error::HttpStatus { status, url },
    }
}

/// The rate-limit signals read off a response, as raw header strings.
///
/// Borrowed rather than parsed so [`classify_status`] is a pure function of the header text and can
/// be exercised from synthetic values without a live response or a `HeaderMap`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RateLimitSignals<'a> {
    /// `x-ratelimit-remaining` (github/gitea/gitee) or `RateLimit-Remaining` (gitlab).
    pub(crate) remaining: Option<&'a str>,
    /// `x-ratelimit-reset` / `RateLimit-Reset`: the unix timestamp at which the quota resets.
    pub(crate) reset: Option<&'a str>,
    /// `Retry-After`: the delay the server asks the client to wait.
    pub(crate) retry_after: Option<&'a str>,
}

/// The ceiling applied to every server-supplied wait (`Retry-After`, and `reset_at` measured from
/// now): 24 hours.
///
/// These values are attacker-controlled — anything able to shape the response picks them — and the
/// documented use for them is "sleep this long before retrying", so an unbounded value is a way to
/// switch off a caller's update channel (and with it its security updates) permanently. A wait past
/// the ceiling is treated as absent (`None`) rather than clamped down to the ceiling, so a caller
/// falls back to its own policy instead of trusting a nonsense number. 24h is comfortably above any
/// real forge window (GitHub's is an hour) while staying well short of "indefinitely".
const MAX_RATE_LIMIT_WAIT: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Map a status + its rate-limit headers to an error variant.
///
/// - **429** is always [`Error::RateLimited`]: RFC 6585 defines the status as rate limiting, and a
///   bare 429 with no quota headers at all is what proxies, CDNs, and self-hosted gitea return.
///   The header-blind [`status_to_error`] classifies it the same way (this path only adds the
///   `reset_at` / `retry_after` wait fields).
/// - **403** is [`Error::RateLimited`] when the response reports a spent quota
///   (remaining-quota header parsing to `0`) *or* supplies a usable `Retry-After`. The latter is
///   GitHub's *secondary* rate limit, which answers 403 + `Retry-After` while
///   `x-ratelimit-remaining` is still nonzero. A bare 403 with neither signal is a genuine
///   authorization failure and stays [`Error::Unauthorized`]. A `Retry-After: 0` does not count as
///   "usable" ([`parse_retry_after`] treats a zero delay as `None`), so a 403 with no other signal
///   stays `Unauthorized` too, rather than becoming a `RateLimited` with a zero-second wait.
/// - Every other status is untouched by the quota headers and falls through to
///   [`status_to_error`].
pub(crate) fn classify_status(status: u16, url: &str, signals: RateLimitSignals<'_>) -> Error {
    let quota_spent = signals
        .remaining
        .and_then(|v| v.trim().parse::<u64>().ok())
        .is_some_and(|remaining| remaining == 0);
    // Parsed up front so the 403 decision keys on the same value the variant carries: a
    // `Retry-After` rejected by the clamp is no signal at all.
    let retry_after = signals.retry_after.and_then(parse_retry_after);
    let rate_limited = match status {
        429 => true,
        403 => quota_spent || retry_after.is_some(),
        _ => false,
    };
    if rate_limited {
        return Error::RateLimited {
            status,
            url: redact_url(url),
            reset_at: signals.reset.and_then(parse_reset_epoch),
            retry_after,
        };
    }
    status_to_error(status, url)
}

/// Parse an `x-ratelimit-reset` / `RateLimit-Reset` value (unix timestamp in seconds) into an
/// instant. `None` for a non-numeric value, one too large to represent, or one further out than
/// [`MAX_RATE_LIMIT_WAIT`] from now. An instant in the past is kept as-is (it renders no wait).
fn parse_reset_epoch(value: &str) -> Option<std::time::SystemTime> {
    let secs: u64 = value.trim().parse().ok()?;
    let at = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(secs))?;
    match at.duration_since(std::time::SystemTime::now()) {
        Ok(wait) if wait > MAX_RATE_LIMIT_WAIT => None,
        _ => Some(at),
    }
}

/// Parse a `Retry-After` value. Only the delta-seconds form is supported; the HTTP-date form
/// returns `None` rather than pulling in a date parser (the reset header already carries an
/// absolute instant). A delay above [`MAX_RATE_LIMIT_WAIT`] is also `None`.
///
/// A **zero** delay is also `None`, not `Some(Duration::ZERO)`: `classify_status`'s 403 branch
/// keys on `retry_after.is_some()`, so a literal `Retry-After: 0` would otherwise promote a bare
/// authorization failure to `Error::RateLimited` carrying a zero-second wait, and a caller
/// following this crate's own documented sleep-then-continue pattern would spin in a tight loop
/// against the server. Treating "wait zero seconds" as "no wait was actually communicated" keeps
/// that 403 as `Error::Unauthorized`. A 429 is unaffected: `classify_status` classifies every 429
/// as rate limiting on the status code alone, regardless of `retry_after`.
fn parse_retry_after(value: &str) -> Option<std::time::Duration> {
    let delay = std::time::Duration::from_secs(value.trim().parse().ok()?);
    (!delay.is_zero() && delay <= MAX_RATE_LIMIT_WAIT).then_some(delay)
}

/// [`status_to_error`] with the response's headers in hand, so a rate-limited response is
/// distinguished from an authorization failure. Used by every built-in HTTP client on the non-2xx
/// path. `HeaderMap` lookups are case-insensitive, so one lookup key covers every casing of a given
/// header name (gitlab's `RateLimit-Remaining` matches `ratelimit-remaining`); the *differently
/// named* github/gitlab headers are covered by the explicit `.or_else` fallbacks below.
///
/// The un-prefixed `ratelimit-reset` fallback is read under the same assumption as
/// `x-ratelimit-reset`: a unix-epoch timestamp, per [`parse_reset_epoch`]. That holds for every
/// forge this crate talks to (gitlab sends unix time under this exact spelling), but
/// draft-ietf-httpapi-ratelimit-headers defines the un-prefixed `RateLimit-Reset` as
/// *delta-seconds from now*, not an absolute epoch. A server following that draft would have its
/// value parsed as an epoch second in 1970, which is always in the past, so `parse_reset_epoch`
/// would not reject it outright but `rate_limit_delay()` would silently derive no wait from it
/// (a past instant yields no duration). No supported forge exhibits this, so the fallback is not
/// changed here; this is only a warning for a future backend that speaks the draft dialect.
pub(crate) fn status_to_error_with_headers(
    status: u16,
    url: &str,
    headers: &http::HeaderMap,
) -> Error {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    classify_status(
        status,
        url,
        RateLimitSignals {
            remaining: get("x-ratelimit-remaining").or_else(|| get("ratelimit-remaining")),
            reset: get("x-ratelimit-reset").or_else(|| get("ratelimit-reset")),
            retry_after: get("retry-after"),
        },
    )
}

/// Redact sensitive query-parameter values from a URL for display/logging. Blanks the value of any
/// `X-Amz-Signature` (a live capability until expiry) and `X-Amz-Credential` (the access-key id) so
/// a presigned s3 URL is safe to surface. Non-s3 URLs are returned unchanged.
pub(crate) fn redact_url(url: &str) -> String {
    let mut out = url.to_string();
    for key in ["X-Amz-Signature", "X-Amz-Credential"] {
        let needle = format!("{key}=");
        if let Some(start) = out.find(&needle) {
            let val_start = start + needle.len();
            let val_end = out[val_start..]
                .find('&')
                .map(|i| val_start + i)
                .unwrap_or(out.len());
            out.replace_range(val_start..val_end, "REDACTED");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Error, MessageError};
    use std::error::Error as _;

    #[test]
    fn redact_url_blanks_amz_signature_and_credential() {
        let url = "https://bucket.s3.amazonaws.com/app.tar.gz?X-Amz-Credential=AKIAEXAMPLE%2F20260101\
                   &X-Amz-Expires=300&X-Amz-Signature=deadbeefcafe&X-Amz-SignedHeaders=host";
        let red = super::redact_url(url);
        assert!(
            !red.contains("deadbeefcafe"),
            "the signature value must be redacted: {red}"
        );
        assert!(
            !red.contains("AKIAEXAMPLE"),
            "the credential value must be redacted: {red}"
        );
        assert!(
            red.contains("X-Amz-Expires=300"),
            "non-sensitive params must be preserved: {red}"
        );
    }

    #[test]
    fn redact_url_leaves_plain_url_unchanged() {
        let url = "https://api.github.com/repos/o/r/releases/assets/1";
        assert_eq!(super::redact_url(url), url);
    }

    #[test]
    fn status_to_error_stores_redacted_url() {
        let err = super::status_to_error(
            403,
            "https://bucket.s3.amazonaws.com/x?X-Amz-Signature=secretsig",
        );
        assert!(
            !err.url().unwrap().contains("secretsig"),
            "status_to_error must store a redacted url"
        );
    }

    /// Produce a real `serde_json::Error` by parsing malformed JSON.
    fn json_error() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("{").unwrap_err()
    }

    /// Produce a real `semver::Error` by parsing an invalid requirement.
    fn semver_error() -> semver::Error {
        "not a version".parse::<semver::Version>().unwrap_err()
    }

    // `Error::Json` is opaque (boxed). The `From<serde_json::Error>` conversion must produce an
    // `Error::Json` whose `source()` surfaces the underlying boxed error, mirroring `Transport`/`S3Auth`.
    // Previously this variant held a concrete `serde_json::Error` (still `source()`-able, but not
    // boxed); after boxing the `source()` arm must deref the box (`&**e`).
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

    // the boxed `Error::Json` must still render with the `JsonError:` Display prefix and embed
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

    // `Error::SemVer` is opaque (boxed) and surfaces its source via the dereferenced box.
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

    // the boxed `Error::SemVer` keeps the `SemVerError:` Display prefix and inner message.
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

    // `Error::Zip` is opaque (boxed). The `From<ZipError>` conversion must produce an
    // `Error::Zip` whose `source()` surfaces the underlying boxed error, mirroring `Transport`/`S3Auth`.
    // Previously this variant held a concrete `zip::result::ZipError` and exposed no `source()`.
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

    // the boxed `Error::Zip` must still render with the `ZipError:` Display prefix and embed
    // the inner error's message. Only `source()` was asserted before boxing; this pins that the
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

    // `Error::Signature` is opaque (boxed) and surfaces its source. Previously it held a concrete
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

    // the boxed `Error::Signature` must still render with the `SignatureError:` Display prefix
    // and embed the inner error's message. Pins that the Display arm dereferences the box.
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

    // the signatures-gated non-UTF8 variant is named `SignatureNonUTF8` (was `NonUTF8`).
    // Naming + Display are pinned here; if the variant were renamed this would not compile.
    // Display prefix is "SignatureError: ..." for consistency with all other variants.
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

    /// Build the rate-limit signals a response would carry, for the classification tests.
    fn signals<'a>(
        remaining: Option<&'a str>,
        reset: Option<&'a str>,
        retry_after: Option<&'a str>,
    ) -> super::RateLimitSignals<'a> {
        super::RateLimitSignals {
            remaining,
            reset,
            retry_after,
        }
    }

    // AUTH-2-2: a 403 carrying a spent quota (`x-ratelimit-remaining: 0`) is rate limiting, not an
    // authorization failure. The reset header is parsed into an absolute instant and `Retry-After`
    // into a delay, so a caller can back off past the window instead of guessing.
    #[test]
    fn classify_status_maps_a_spent_quota_403_to_rate_limited() {
        let err = super::classify_status(
            403,
            "https://api.github.com/repos/o/r/releases/latest",
            signals(Some("0"), Some("1780000000"), Some("60")),
        );
        let Error::RateLimited {
            status,
            url,
            reset_at,
            retry_after,
        } = err
        else {
            panic!("a 403 with remaining=0 must classify as RateLimited, got {err:?}");
        };
        assert_eq!(status, 403);
        assert_eq!(url, "https://api.github.com/repos/o/r/releases/latest");
        assert_eq!(
            reset_at,
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_780_000_000)),
            "x-ratelimit-reset must parse as a unix timestamp"
        );
        assert_eq!(retry_after, Some(std::time::Duration::from_secs(60)));
    }

    // The same 403 WITHOUT the quota headers is a genuine credential failure and must keep its
    // historical `Unauthorized` classification -- this is the distinction the variant exists for.
    #[test]
    fn classify_status_keeps_a_plain_403_unauthorized() {
        let err = super::classify_status(403, "https://example.com/r", signals(None, None, None));
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "a 403 with no rate-limit headers must stay Unauthorized, got {err:?}"
        );
    }

    // Quota remaining but not exhausted, and no `Retry-After` asking the client to back off: the
    // 403 is about *this* request's credentials, not the budget, so it stays `Unauthorized`.
    #[test]
    fn classify_status_keeps_403_unauthorized_when_quota_remains() {
        let err = super::classify_status(
            403,
            "https://example.com/r",
            signals(Some("57"), Some("1780000000"), None),
        );
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "a nonzero remaining quota must not classify as RateLimited, got {err:?}"
        );
    }

    // A 429 with a spent quota is rate limiting too. Gitlab spells the header `RateLimit-Remaining`
    // (no `x-` prefix); `status_to_error_with_headers` maps both spellings onto `remaining`.
    #[test]
    fn classify_status_maps_a_spent_quota_429_to_rate_limited() {
        let err = super::classify_status(
            429,
            "https://gitlab.com/api/v4/x",
            signals(Some("0"), None, None),
        );
        assert!(
            matches!(err, Error::RateLimited { status: 429, .. }),
            "a 429 with remaining=0 must classify as RateLimited, got {err:?}"
        );
    }

    // RFC 6585 defines 429 as rate limiting, so the status alone is enough -- proxies, CDNs and
    // self-hosted gitea all return a bare 429 with no quota headers at all. Classifying that as the
    // catch-all `HttpStatus` (the pre-fix behavior) hid the one case a caller can act on.
    #[test]
    fn classify_status_maps_a_bare_429_to_rate_limited() {
        let err = super::classify_status(429, "https://example.com/r", signals(None, None, None));
        assert!(
            matches!(
                err,
                Error::RateLimited {
                    status: 429,
                    reset_at: None,
                    retry_after: None,
                    ..
                }
            ),
            "a 429 must classify as RateLimited even with no quota headers, got {err:?}"
        );
    }

    // A 429 that carries only a `Retry-After` is still rate limiting, and the delay is picked up.
    #[test]
    fn classify_status_maps_a_429_with_only_retry_after_to_rate_limited() {
        let err = super::classify_status(
            429,
            "https://example.com/r",
            signals(None, None, Some("30")),
        );
        assert!(
            matches!(
                err,
                Error::RateLimited {
                    status: 429,
                    retry_after: Some(d),
                    ..
                } if d == std::time::Duration::from_secs(30)
            ),
            "a 429 with Retry-After must classify as RateLimited carrying the delay, got {err:?}"
        );
    }

    // GitHub's *secondary* rate limit answers 403 + `Retry-After` while `x-ratelimit-remaining` is
    // still NONZERO. Keying only on a spent quota misfiled that as `Unauthorized`, i.e. "your token
    // is wrong" for a caller who merely needs to wait.
    #[test]
    fn classify_status_maps_a_403_with_retry_after_to_rate_limited() {
        let err = super::classify_status(
            403,
            "https://api.github.com/repos/o/r/releases",
            signals(Some("4999"), None, Some("60")),
        );
        assert!(
            matches!(
                err,
                Error::RateLimited {
                    status: 403,
                    retry_after: Some(d),
                    ..
                } if d == std::time::Duration::from_secs(60)
            ),
            "a 403 with Retry-After must classify as RateLimited, got {err:?}"
        );
    }

    // The other 403 signal, unchanged: a spent quota with no `Retry-After` at all.
    #[test]
    fn classify_status_maps_a_403_with_spent_quota_and_no_retry_after_to_rate_limited() {
        let err = super::classify_status(
            403,
            "https://api.github.com/repos/o/r/releases",
            signals(Some("0"), None, None),
        );
        assert!(
            matches!(err, Error::RateLimited { status: 403, .. }),
            "a 403 with remaining=0 must classify as RateLimited, got {err:?}"
        );
    }

    // Only 403/429 consult the quota headers: a 404 carrying a full set of rate-limit headers --
    // including the newly-honored `Retry-After` -- is still a missing resource.
    #[test]
    fn classify_status_ignores_quota_headers_on_a_404() {
        let err = super::classify_status(
            404,
            "https://example.com/r",
            signals(Some("0"), Some("1780000000"), Some("60")),
        );
        assert!(
            matches!(err, Error::NotFound { .. }),
            "a 404 must stay NotFound regardless of quota headers, got {err:?}"
        );
    }

    // A spent quota reported on an unrelated status (e.g. 404) must not be re-classified: only
    // 403/429 are rate-limit statuses.
    #[test]
    fn classify_status_ignores_quota_headers_on_other_statuses() {
        let err =
            super::classify_status(404, "https://example.com/r", signals(Some("0"), None, None));
        assert!(
            matches!(err, Error::NotFound { .. }),
            "a 404 must stay NotFound regardless of quota headers, got {err:?}"
        );
    }

    // Unparseable header values degrade gracefully: the classification still fires off `remaining`,
    // and the fields that could not be parsed are `None` rather than a panic or a bogus instant.
    // `Retry-After` also accepts an HTTP-date form, which is deliberately not parsed.
    #[test]
    fn classify_status_tolerates_unparseable_reset_and_retry_after() {
        let err = super::classify_status(
            403,
            "https://example.com/r",
            signals(
                Some("0"),
                Some("not-a-number"),
                Some("Wed, 21 Oct 2026 07:28:00 GMT"),
            ),
        );
        let Error::RateLimited {
            reset_at,
            retry_after,
            ..
        } = err
        else {
            panic!("expected RateLimited, got {err:?}");
        };
        assert_eq!(
            reset_at, None,
            "a non-numeric reset must not yield an instant"
        );
        assert_eq!(
            retry_after, None,
            "the HTTP-date form of Retry-After is not parsed"
        );
    }

    // The header-reading wrapper: `HeaderMap` matches a name case-insensitively, which is what lets
    // gitlab's `RateLimit-Remaining` be found under the `ratelimit-remaining` lookup key; the
    // *different* github and gitlab names are bridged by the explicit `.or_else` fallback chain.
    // Both spellings must land on the same signal.
    #[test]
    fn status_to_error_with_headers_reads_both_header_spellings() {
        let mut github = http::HeaderMap::new();
        github.insert("x-ratelimit-remaining", "0".parse().unwrap());
        github.insert("x-ratelimit-reset", "1780000000".parse().unwrap());
        assert!(
            matches!(
                super::status_to_error_with_headers(403, "https://api.github.com/x", &github),
                Error::RateLimited {
                    status: 403,
                    reset_at: Some(_),
                    ..
                }
            ),
            "the x-ratelimit-* spelling must classify as RateLimited"
        );

        let mut gitlab = http::HeaderMap::new();
        gitlab.insert("RateLimit-Remaining", "0".parse().unwrap());
        assert!(
            matches!(
                super::status_to_error_with_headers(429, "https://gitlab.com/x", &gitlab),
                Error::RateLimited { status: 429, .. }
            ),
            "the un-prefixed gitlab spelling must classify as RateLimited"
        );

        assert!(
            matches!(
                super::status_to_error_with_headers(
                    403,
                    "https://example.com/x",
                    &http::HeaderMap::new()
                ),
                Error::Unauthorized { status: 403, .. }
            ),
            "a header-less 403 must stay Unauthorized"
        );
    }

    // The URL stored on a `RateLimited` is redacted like every other HTTP variant's, so a presigned
    // URL's signature cannot leak through `url()` or the Display string.
    #[test]
    fn classify_status_redacts_the_rate_limited_url() {
        let err = super::classify_status(
            429,
            "https://bucket.s3.amazonaws.com/x?X-Amz-Signature=secretsig",
            signals(Some("0"), None, None),
        );
        assert!(
            !err.url().unwrap().contains("secretsig"),
            "the RateLimited url must be redacted"
        );
    }

    // AUTH-2-3: `http_status()` / `url()` work for `RateLimited` exactly as for the other HTTP
    // variants, so a caller keying on status/URL does not need to learn a new accessor.
    #[test]
    fn http_status_and_url_helpers_cover_rate_limited() {
        let err = Error::RateLimited {
            status: 429,
            url: "https://example.com/r".to_string(),
            reset_at: None,
            retry_after: None,
        };
        assert_eq!(err.http_status(), Some(429));
        assert_eq!(err.url(), Some("https://example.com/r"));
        assert!(
            err.source().is_none(),
            "RateLimited carries no chained source"
        );
    }

    // AUTH-2-4: the Display string must name rate limiting and the token remedy rather than reading
    // as an auth failure, and must surface the wait when the response supplied one.
    #[test]
    fn rate_limited_display_names_the_limit_and_the_remedy() {
        let err = Error::RateLimited {
            status: 403,
            url: "https://api.github.com/repos/o/r/releases/latest".to_string(),
            reset_at: None,
            retry_after: Some(std::time::Duration::from_secs(90)),
        };
        let msg = err.to_string();
        assert!(
            msg.starts_with("RateLimitedError:"),
            "the Display string must identify the variant, got: {msg}"
        );
        assert!(
            msg.contains("rate limited"),
            "the message must name rate limiting, not authorization, got: {msg}"
        );
        assert!(
            !msg.contains("not authorized"),
            "the message must not read as an auth failure, got: {msg}"
        );
        assert!(
            msg.contains("retry in 90s"),
            "the message must surface the wait, got: {msg}"
        );
        assert!(
            msg.contains("auth token"),
            "the message must name the token remedy, got: {msg}"
        );
    }

    // A `Retry-After`-sourced wait is a requested back-off, not proof the quota is exhausted
    // (GitHub's secondary rate limit answers 403 + `Retry-After` while the primary quota is still
    // nonzero), so it must not be worded as "quota resets in Ns".
    #[test]
    fn rate_limited_display_does_not_call_a_retry_after_wait_a_quota_reset() {
        let msg = Error::RateLimited {
            status: 403,
            url: "https://api.github.com/x".to_string(),
            reset_at: None,
            retry_after: Some(std::time::Duration::from_secs(30)),
        }
        .to_string();
        assert!(
            !msg.contains("quota resets"),
            "a Retry-After-sourced wait must not be worded as a quota reset, got: {msg}"
        );
    }

    // A wait derived from `reset_at` (no `Retry-After` in hand) IS a quota-window reset, so it
    // keeps the "quota resets in Ns" wording -- only the `Retry-After` path changed under B10.
    #[test]
    fn rate_limited_display_still_calls_a_reset_at_wait_a_quota_reset() {
        let msg = Error::RateLimited {
            status: 403,
            url: "https://api.github.com/x".to_string(),
            reset_at: Some(std::time::SystemTime::now() + std::time::Duration::from_secs(600)),
            retry_after: None,
        }
        .to_string();
        assert!(
            msg.contains("quota resets in"),
            "a reset_at-derived wait must still read as a quota reset, got: {msg}"
        );
        assert!(
            !msg.contains("retry in"),
            "a reset_at-derived wait must not use the Retry-After wording, got: {msg}"
        );
    }

    // B10: when BOTH fields are present the Display branches on `retry_after` alone (it is
    // matched directly, not routed through `rate_limit_delay()`), so it must render "retry in Ns"
    // using the `Retry-After` value verbatim and must NOT fall back to the `reset_at`-derived
    // wording or value, even though `rate_limit_delay()` would resolve to the same number here.
    #[test]
    fn rate_limited_display_prefers_retry_after_wording_when_both_fields_are_present() {
        let msg = Error::RateLimited {
            status: 403,
            url: "https://api.github.com/x".to_string(),
            // A reset_at far enough out that, if the Display mistakenly rendered a reset-derived
            // wait instead of the Retry-After value, the two numbers would visibly disagree.
            reset_at: Some(std::time::SystemTime::now() + std::time::Duration::from_secs(3600)),
            retry_after: Some(std::time::Duration::from_secs(30)),
        }
        .to_string();
        assert!(
            msg.contains("retry in 30s"),
            "with both fields present the Retry-After value must be rendered, got: {msg}"
        );
        assert!(
            !msg.contains("quota resets"),
            "with both fields present the message must not also/instead read as a quota reset, \
             got: {msg}"
        );
        assert!(
            !msg.contains("3600s"),
            "the reset_at-derived duration must never leak into the message, got: {msg}"
        );
    }

    // With no wait information at all the message still renders (no dangling clause), and an
    // already-elapsed reset instant does not produce a bogus countdown.
    #[test]
    fn rate_limited_display_omits_the_wait_when_unknown_or_elapsed() {
        let no_info = Error::RateLimited {
            status: 403,
            url: "https://example.com/r".to_string(),
            reset_at: None,
            retry_after: None,
        }
        .to_string();
        assert!(
            !no_info.contains("resets in"),
            "with no reset info the wait clause must be omitted, got: {no_info}"
        );
        assert!(
            no_info.ends_with("check less often"),
            "the message must still end with the remedy, got: {no_info}"
        );

        let elapsed = Error::RateLimited {
            status: 403,
            url: "https://example.com/r".to_string(),
            // An instant well in the past: `duration_since(now)` errors, so no clause is rendered.
            reset_at: Some(std::time::UNIX_EPOCH),
            retry_after: None,
        }
        .to_string();
        assert!(
            !elapsed.contains("resets in"),
            "an elapsed reset window must not render a countdown, got: {elapsed}"
        );
    }

    // The Display string's clauses are separated consistently (`; `), not a comma for the wait and
    // a colon for the remedy. Pinned here so the whole rendered shape lives in one place.
    #[test]
    fn rate_limited_display_separates_its_clauses_consistently() {
        let msg = Error::RateLimited {
            status: 403,
            url: "https://api.github.com/x".to_string(),
            reset_at: None,
            retry_after: Some(std::time::Duration::from_secs(90)),
        }
        .to_string();
        assert_eq!(
            msg,
            "RateLimitedError: request to https://api.github.com/x was rate limited (HTTP 403); \
             retry in 90s; set an auth token to raise the limit, or check less often",
            "the RateLimited Display shape changed"
        );
    }

    /// The `x-ratelimit-reset` header value for a window resetting `secs_from_now` seconds from
    /// now. `now` is truncated to a whole second, so the wait the parser actually computes is at
    /// most `secs_from_now` — a "exactly at the ceiling" case can never tip over it.
    fn reset_header(secs_from_now: u64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock is after the unix epoch")
            .as_secs();
        (now + secs_from_now).to_string()
    }

    // A `reset_at` inside the 24h ceiling is kept, and so is one in the *past* (the clamp only
    // rejects absurd futures; an elapsed window is already handled by yielding no wait).
    #[test]
    fn parse_reset_epoch_keeps_a_normal_and_a_past_window() {
        let soon = reset_header(600);
        assert_eq!(
            super::parse_reset_epoch(&soon),
            Some(
                std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(soon.parse::<u64>().unwrap())
            ),
            "a window 10 minutes out must be kept as-is"
        );
        assert_eq!(
            super::parse_reset_epoch("1"),
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)),
            "a reset instant in the past must not be rejected by the future-facing clamp"
        );
    }

    // Exactly at the 24h ceiling is still accepted: the clamp rejects "further out than", not
    // "at".
    #[test]
    fn parse_reset_epoch_keeps_a_window_exactly_at_the_ceiling() {
        assert!(
            super::parse_reset_epoch(&reset_header(24 * 60 * 60)).is_some(),
            "a reset exactly 24h out must be honored"
        );
    }

    // Past the ceiling the value is dropped rather than carried: a server that claims the quota
    // resets in a week would otherwise park the caller for a week. (The header is built two
    // seconds past the ceiling so truncating `now` to a whole second cannot pull it back under.)
    #[test]
    fn parse_reset_epoch_rejects_a_window_past_the_ceiling() {
        assert_eq!(
            super::parse_reset_epoch(&reset_header(24 * 60 * 60 + 2)),
            None,
            "a reset more than 24h out must be discarded"
        );
        assert_eq!(
            super::parse_reset_epoch(&reset_header(30 * 24 * 60 * 60)),
            None,
            "a reset a month out must be discarded"
        );
    }

    // The degenerate maximum: `u64::MAX` seconds since the epoch must not survive as an instant a
    // caller would sleep until.
    #[test]
    fn parse_reset_epoch_rejects_the_u64_max_timestamp() {
        assert_eq!(
            super::parse_reset_epoch(&u64::MAX.to_string()),
            None,
            "a u64::MAX reset timestamp must not yield an instant"
        );
    }

    // Malformed header text must degrade to `None`, never panic: empty, whitespace-only,
    // non-numeric, a negative number (the field is a `u64` timestamp, so a leading `-` is simply
    // unparseable rather than "before the epoch"), and a float.
    #[test]
    fn parse_reset_epoch_rejects_malformed_values() {
        for bad in ["", "   ", "not-a-number", "-1", "1.5", "0x10"] {
            assert_eq!(
                super::parse_reset_epoch(bad),
                None,
                "malformed reset value {bad:?} must yield None, not a panic or a bogus instant"
            );
        }
    }

    // A `Retry-After` inside the ceiling is taken at face value.
    #[test]
    fn parse_retry_after_keeps_a_normal_delay() {
        assert_eq!(
            super::parse_retry_after("60"),
            Some(std::time::Duration::from_secs(60)),
            "a 60s Retry-After must be kept"
        );
    }

    // The floor is "zero is no signal", not "small is no signal": the smallest nonzero delay is
    // the boundary directly above the one A4 carved out, and must still be honored as a real wait.
    #[test]
    fn parse_retry_after_keeps_the_smallest_nonzero_delay() {
        assert_eq!(
            super::parse_retry_after("1"),
            Some(std::time::Duration::from_secs(1)),
            "a 1s Retry-After is the boundary directly above the zero floor and must be kept"
        );
    }

    // Surrounding whitespace is trimmed before parsing, matching `first_env_token`'s behavior on
    // the analogous header-adjacent value elsewhere in the crate.
    #[test]
    fn parse_retry_after_trims_surrounding_whitespace() {
        assert_eq!(
            super::parse_retry_after(" 60 \t"),
            Some(std::time::Duration::from_secs(60)),
            "surrounding whitespace around a valid delay must be trimmed, not treated as unparseable"
        );
    }

    // Malformed header text must degrade to `None`, never panic: empty, whitespace-only,
    // non-numeric, a negative number (the field is a `u64` delay, so a leading `-` is simply
    // unparseable), and a float. The HTTP-date form is covered separately by
    // `classify_status_tolerates_unparseable_reset_and_retry_after`.
    #[test]
    fn parse_retry_after_rejects_malformed_values() {
        for bad in ["", "   ", "not-a-number", "-1", "1.5", "0x10"] {
            assert_eq!(
                super::parse_retry_after(bad),
                None,
                "malformed Retry-After value {bad:?} must yield None, not a panic or a bogus delay"
            );
        }
    }

    // A4: a literal `Retry-After: 0` is no signal at all, not a zero-second wait. `classify_status`
    // keys its 403 promotion on `retry_after.is_some()`, so `Some(Duration::ZERO)` here would turn a
    // bare authorization failure into a `RateLimited` a caller's documented sleep-then-continue
    // pattern would spin on forever.
    #[test]
    fn parse_retry_after_treats_a_zero_delay_as_no_signal() {
        assert_eq!(
            super::parse_retry_after("0"),
            None,
            "a zero-second Retry-After must not be carried through as Some(0)"
        );
    }

    // Exactly 24h is the largest delay still honored; one second more is discarded.
    #[test]
    fn parse_retry_after_clamps_at_twenty_four_hours() {
        assert_eq!(
            super::parse_retry_after("86400"),
            Some(std::time::Duration::from_secs(86_400)),
            "a Retry-After exactly at the 24h ceiling must be honored"
        );
        assert_eq!(
            super::parse_retry_after("86401"),
            None,
            "a Retry-After one second past the ceiling must be discarded"
        );
    }

    // The attack this clamp exists for: `Retry-After: 18446744073709551615` is ~584 billion years,
    // which would permanently disable a caller's (security) update channel if it were honored.
    #[test]
    fn parse_retry_after_rejects_the_u64_max_delay() {
        assert_eq!(
            super::parse_retry_after(&u64::MAX.to_string()),
            None,
            "a u64::MAX Retry-After must not yield a delay"
        );
    }

    // The clamp is applied *before* the 403 decision, so an over-ceiling `Retry-After` is no signal
    // at all: it must not promote a 403 to `RateLimited` while carrying `retry_after: None`.
    #[test]
    fn classify_status_ignores_an_over_ceiling_retry_after_on_a_403() {
        let err = super::classify_status(
            403,
            "https://example.com/r",
            signals(Some("4999"), None, Some(&u64::MAX.to_string())),
        );
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "an unusable Retry-After must not promote a 403 to RateLimited, got {err:?}"
        );
    }

    // A4: `Retry-After: 0` on a 403 with no other rate-limit signal is a genuine authorization
    // failure, not a zero-wait rate limit. Before the fix this promoted to `RateLimited` with
    // `rate_limit_delay() == Some(Duration::ZERO)`.
    #[test]
    fn classify_status_keeps_a_403_with_zero_retry_after_unauthorized() {
        let err =
            super::classify_status(403, "https://example.com/r", signals(None, None, Some("0")));
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "a 403 with Retry-After: 0 and no spent-quota signal must stay Unauthorized, got {err:?}"
        );
    }

    // The same 403 + zero `Retry-After`, but this time reported net of an already-spent quota:
    // the spent-quota signal alone is still enough to classify as RateLimited, `retry_after` just
    // carries no wait.
    #[test]
    fn classify_status_still_rate_limits_a_spent_quota_403_with_zero_retry_after() {
        let err = super::classify_status(
            403,
            "https://example.com/r",
            signals(Some("0"), None, Some("0")),
        );
        assert!(
            matches!(
                err,
                Error::RateLimited {
                    status: 403,
                    retry_after: None,
                    ..
                }
            ),
            "a spent quota must still classify as RateLimited even with Retry-After: 0, got {err:?}"
        );
    }

    // `reset_at` is enrichment, never a promotion signal on its own: a 403 that supplies only a
    // (perfectly valid, future) `x-ratelimit-reset` and neither a spent quota nor a `Retry-After`
    // must stay `Unauthorized`, and the reset value must be dropped along with it (`status_to_error`
    // carries no reset field). A regression that added `signals.reset.is_some()` to the 403
    // promotion condition would turn every 403 that merely echoes a reset header into a rate limit.
    #[test]
    fn classify_status_keeps_a_403_unauthorized_when_only_reset_at_is_present() {
        let err = super::classify_status(
            403,
            "https://example.com/r",
            signals(None, Some(&reset_header(600)), None),
        );
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "a 403 with only a reset_at header (no spent quota, no Retry-After) must stay \
             Unauthorized, got {err:?}"
        );
    }

    // A4 must not touch 429: RFC 6585 defines the status itself as rate limiting, independent of
    // any header, so `Retry-After: 0` on a 429 stays RateLimited (just with no usable wait).
    #[test]
    fn classify_status_keeps_a_429_with_zero_retry_after_rate_limited() {
        let err =
            super::classify_status(429, "https://example.com/r", signals(None, None, Some("0")));
        assert!(
            matches!(
                err,
                Error::RateLimited {
                    status: 429,
                    retry_after: None,
                    ..
                }
            ),
            "a 429 must stay RateLimited regardless of Retry-After, got {err:?}"
        );
    }

    // `rate_limit_delay` is the single implementation of the back-off precedence: the server's
    // explicit `Retry-After` wins over a `reset_at`-derived wait when both are present.
    #[test]
    fn rate_limit_delay_prefers_retry_after_over_reset_at() {
        let err = Error::RateLimited {
            status: 403,
            url: "https://example.com/r".to_string(),
            reset_at: Some(std::time::SystemTime::now() + std::time::Duration::from_secs(3600)),
            retry_after: Some(std::time::Duration::from_secs(30)),
        };
        assert_eq!(
            err.rate_limit_delay(),
            Some(std::time::Duration::from_secs(30)),
            "Retry-After must take precedence over the reset instant"
        );
    }

    // With only a `Retry-After` the delay is that header, verbatim.
    #[test]
    fn rate_limit_delay_uses_retry_after_alone() {
        let err = Error::RateLimited {
            status: 429,
            url: "https://example.com/r".to_string(),
            reset_at: None,
            retry_after: Some(std::time::Duration::from_secs(45)),
        };
        assert_eq!(
            err.rate_limit_delay(),
            Some(std::time::Duration::from_secs(45))
        );
    }

    // GitHub's *primary* limit sends only `x-ratelimit-reset`, so the fallback matters: a caller
    // reaching for `retry_after.unwrap_or_default()` would sleep zero and burn more quota.
    #[test]
    fn rate_limit_delay_derives_a_wait_from_a_future_reset_at() {
        let err = Error::RateLimited {
            status: 403,
            url: "https://api.github.com/x".to_string(),
            reset_at: Some(std::time::SystemTime::now() + std::time::Duration::from_secs(600)),
            retry_after: None,
        };
        let delay = err
            .rate_limit_delay()
            .expect("a future reset instant must yield a wait");
        assert!(
            delay <= std::time::Duration::from_secs(600)
                && delay > std::time::Duration::from_secs(590),
            "the wait must be measured from now, got {delay:?}"
        );
    }

    // An already-elapsed window yields `None` rather than panicking the way
    // `duration_since(now).unwrap()` would.
    #[test]
    fn rate_limit_delay_is_none_for_an_elapsed_reset_at() {
        let err = Error::RateLimited {
            status: 403,
            url: "https://example.com/r".to_string(),
            reset_at: Some(std::time::UNIX_EPOCH),
            retry_after: None,
        };
        assert_eq!(
            err.rate_limit_delay(),
            None,
            "an elapsed reset window must yield no wait, not a panic or a bogus duration"
        );
    }

    // No signal at all: `None` means "nothing known", not "wait zero".
    #[test]
    fn rate_limit_delay_is_none_when_nothing_is_known() {
        let err = Error::RateLimited {
            status: 429,
            url: "https://example.com/r".to_string(),
            reset_at: None,
            retry_after: None,
        };
        assert_eq!(err.rate_limit_delay(), None);
    }

    // Every other variant answers `None`, so a caller can call it unconditionally on an `Error`.
    #[test]
    fn rate_limit_delay_is_none_for_a_non_rate_limited_variant() {
        assert_eq!(
            Error::Unauthorized {
                status: 403,
                url: "https://example.com/r".to_string(),
            }
            .rate_limit_delay(),
            None,
            "Unauthorized carries no rate-limit wait"
        );
        assert_eq!(
            Error::NotFound {
                url: "https://example.com/r".to_string(),
            }
            .rate_limit_delay(),
            None,
            "NotFound carries no rate-limit wait"
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

    // The header-blind path used to disagree with `classify_status` about 429: it fell through to
    // the `_ =>` arm and produced `HttpStatus { status: 429 }`, while the header-aware path (and the
    // `HttpStatus` variant docs) said a 429 is ALWAYS `RateLimited`. Both could not be true for a
    // downstream custom `HttpClient` calling `Error::http_status_error`, or for ureq's headerless
    // `StatusCode` fallback arm. 429 means rate limiting by RFC 6585 regardless of headers, so this
    // path classifies it too -- carrying no wait, since it cannot see the headers that supply one.
    #[test]
    fn status_to_error_maps_429_to_rate_limited_without_a_wait() {
        let e = super::status_to_error(429, "https://example.com/r");
        assert!(
            matches!(
                e,
                Error::RateLimited {
                    status: 429,
                    ref url,
                    reset_at: None,
                    retry_after: None,
                } if url == "https://example.com/r"
            ),
            "status 429 must map to Error::RateLimited with no wait fields, got {:?}",
            e
        );
        assert_eq!(
            e.rate_limit_delay(),
            None,
            "a header-blind 429 knows no wait, so rate_limit_delay must be None"
        );
        assert_eq!(e.http_status(), Some(429));
    }

    // The other half of the same boundary: only 429 moved. 401/403 stay `Unauthorized` on the
    // header-blind path, because a 403 needs a header to tell a spent quota from a bad credential
    // (that distinction is the whole reason `RateLimited` is a separate variant).
    #[test]
    fn status_to_error_keeps_401_and_403_unauthorized() {
        assert!(matches!(
            super::status_to_error(401, "https://example.com/r"),
            Error::Unauthorized { status: 401, .. }
        ));
        assert!(matches!(
            super::status_to_error(403, "https://example.com/r"),
            Error::Unauthorized { status: 403, .. }
        ));
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
            shown.contains("zip") && shown.contains("archive-zip"),
            "ArchiveNotEnabled Display must contain the extension and the feature name, got: {}",
            shown
        );
        // Message style matches the other variants: lowercase after the prefix, no trailing
        // punctuation.
        assert!(
            !shown.ends_with('!') && !shown.ends_with('.'),
            "ArchiveNotEnabled Display must not end with punctuation, got: {}",
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

    // --- structured-variant unit tests ----------------------------------------------------

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

    // `InstallPathNotWritable` Display names the path and suggests elevated privileges or a
    // user-writable bin_install_path. It carries no source and exposes no http_status()/url().
    #[test]
    fn install_path_not_writable_display_and_no_source() {
        let err = Error::InstallPathNotWritable {
            path: std::path::PathBuf::from("/usr/local/bin/app"),
        };
        let shown = err.to_string();
        assert!(
            shown.starts_with("InstallPathNotWritableError: "),
            "InstallPathNotWritable Display must keep the greppable prefix, got: {shown}"
        );
        assert!(
            shown.contains("/usr/local/bin/app"),
            "InstallPathNotWritable Display must name the path, got: {shown}"
        );
        // The remedy is named without naming a specific setter: the same variant covers
        // `bin_install_path` and, in bundle mode, `bundle_install_path` (or its parent).
        assert!(
            shown.contains("elevated privileges") && shown.contains("user-writable install path"),
            "InstallPathNotWritable Display must suggest elevated privileges or a user-writable \
             install path, got: {shown}"
        );
        assert!(
            err.source().is_none(),
            "InstallPathNotWritable carries no source, got {:?}",
            err.source()
        );
        assert_eq!(err.http_status(), None);
        assert_eq!(err.url(), None);
    }

    // `InstallPathNotWritable` is `#[non_exhaustive]`; a `..`-destructure that reads `path` must
    // compile (adding a field stays non-breaking for downstream matchers).
    #[test]
    fn install_path_not_writable_is_non_exhaustive_struct_variant() {
        let err = Error::InstallPathNotWritable {
            path: std::path::PathBuf::from("/opt/app"),
        };
        let Error::InstallPathNotWritable { path, .. } = err else {
            panic!("expected InstallPathNotWritable");
        };
        assert_eq!(path, std::path::PathBuf::from("/opt/app"));
    }

    // `NoCurrentVersion` is a distinct, self-describing variant (not `MissingField`): its Display
    // names the missing current_version and points at `Update::is_update_available`, carries no
    // source, and exposes no http_status()/url(). Pins that the bare-listing precheck error is not
    // the misleading builder-field message.
    #[test]
    fn no_current_version_display_and_no_source() {
        let err = Error::NoCurrentVersion;
        let shown = err.to_string();
        assert_eq!(
            shown,
            "ReleaseError: this Releases has no current_version to compare against; use \
             `Update::is_update_available` for a configured updater"
        );
        assert!(
            !matches!(err, Error::MissingField { .. }),
            "NoCurrentVersion must be distinct from MissingField"
        );
        assert!(err.source().is_none(), "NoCurrentVersion carries no source");
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
        let err = Error::missing_asset_field("tag_name");
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

    // `ArchiveVerificationRejected` Display, with and without a reason. It names the *archive*, not
    // the new binary: a caller reading only the message must be able to tell which of the two hooks
    // rejected the update.
    #[test]
    fn archive_verification_rejected_display_variants() {
        assert_eq!(
            Error::ArchiveVerificationRejected { reason: None }.to_string(),
            "ArchiveVerificationRejectedError: verification rejected the downloaded archive"
        );
        assert_eq!(
            Error::ArchiveVerificationRejected {
                reason: Some("no attestation found".into())
            }
            .to_string(),
            "ArchiveVerificationRejectedError: verification rejected the downloaded archive: no attestation found"
        );
        assert_eq!(
            Error::ArchiveVerificationRejected { reason: None }.http_status(),
            None
        );
        assert!(
            Error::ArchiveVerificationRejected { reason: None }
                .source()
                .is_none()
        );
    }

    // `ChecksumSourceInvalid` Display names both the artifact a digest was wanted for and why none
    // could be resolved, so the message alone distinguishes "no such sums asset" from "no entry for
    // this artifact" without matching on fields.
    #[test]
    fn checksum_source_invalid_display_names_asset_and_reason() {
        let err = Error::ChecksumSourceInvalid {
            asset: "app-1.0.0.tar.gz".into(),
            reason: "release 1.0.0 has no asset named `SHA256SUMS`".into(),
        };
        assert_eq!(
            err.to_string(),
            "ChecksumSourceInvalidError: could not resolve a checksum for `app-1.0.0.tar.gz`: \
             release 1.0.0 has no asset named `SHA256SUMS`"
        );
        assert_eq!(err.http_status(), None);
        assert!(err.source().is_none());
    }

    // The constructor fills `reason`, mirroring `verification_rejected`.
    #[test]
    fn archive_verification_rejected_constructor_sets_reason() {
        match Error::archive_verification_rejected("nope") {
            Error::ArchiveVerificationRejected { reason } => {
                assert_eq!(reason.as_deref(), Some("nope"));
            }
            other => panic!("expected ArchiveVerificationRejected, got {other:?}"),
        }
    }

    // `InvalidResponse` carries a boxed source and chains it through `source()`.
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

    // `InvalidHeader` carries a boxed source and chains it through `source()`.
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

    // `InvalidAuthToken` carries a boxed source and chains it through `source()`.
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

    // `Internal` with a source chains it; without a source returns None.
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

    // `Io` still carries the concrete `std::io::Error` (not boxed), exposing `ErrorKind`.
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

    // `Error` is `#[non_exhaustive]`, so a downstream-style `match` with a wildcard arm
    // compiles and the struct variants stay non-breaking to add to.
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

    // The `#[non_exhaustive]` struct variants require a trailing `..` to destructure from a
    // downstream perspective (adding a field stays non-breaking). A destructure that binds the
    // current fields plus `..` must compile and read them. This pins the struct-level
    // non_exhaustive contract that the enum-level wildcard test above does not exercise.
    //
    // Variants with `#[non_exhaustive]` on the variant itself (in addition to the enum-level
    // `#[non_exhaustive]`): `Internal`, `VerificationRejected`, `ArchiveVerificationRejected`,
    // `ChecksumSourceInvalid`, `NoReleaseFound`,
    // `MissingAssetField`, `InvalidResponse`, `MissingField`, `InvalidHeader`,
    // `InvalidAuthToken`, `Unauthorized`, `HttpStatus`, `InvalidAssetName`.
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

        // `Unauthorized` is `#[non_exhaustive]`; `..` lets us read just `status`.
        let unauth = Error::Unauthorized {
            status: 401,
            url: "u".into(),
        };
        if let Error::Unauthorized { status, .. } = &unauth {
            assert_eq!(*status, 401);
        } else {
            panic!("expected Unauthorized");
        }

        // `HttpStatus` is `#[non_exhaustive]`; `..` lets us read just `status`.
        let hs = Error::HttpStatus {
            status: 503,
            url: "u".into(),
        };
        if let Error::HttpStatus { status, .. } = &hs {
            assert_eq!(*status, 503);
        } else {
            panic!("expected HttpStatus");
        }

        // `InvalidAssetName` is `#[non_exhaustive]`; `..` lets us read just `name`.
        let ian = Error::InvalidAssetName {
            name: "../etc/passwd".into(),
        };
        if let Error::InvalidAssetName { name, .. } = &ian {
            assert_eq!(name, "../etc/passwd");
        } else {
            panic!("expected InvalidAssetName");
        }
    }

    // Documents that `Unauthorized`, `HttpStatus`, and `InvalidAssetName` carry the
    // `#[non_exhaustive]` attribute on the variant (not only at the enum level). This test
    // asserts observable behaviour: the Display output and field values are accessible through
    // a `..`-pattern, which is what downstream code must use. If any of these variants were
    // removed or renamed, this test would fail to compile.
    #[test]
    fn unauthorized_http_status_invalid_asset_name_are_non_exhaustive_struct_variants() {
        let unauth = Error::Unauthorized {
            status: 403,
            url: "https://api.example.com/releases".into(),
        };
        // Read `status` via the `..`-pattern (models the downstream requirement).
        let Error::Unauthorized { status, .. } = unauth else {
            panic!("expected Unauthorized");
        };
        assert_eq!(status, 403);

        let hs = Error::HttpStatus {
            status: 502,
            url: "https://api.example.com/releases".into(),
        };
        let Error::HttpStatus { status, .. } = hs else {
            panic!("expected HttpStatus");
        };
        assert_eq!(status, 502);

        let ian = Error::InvalidAssetName {
            name: "../../shadow".into(),
        };
        let Error::InvalidAssetName { name, .. } = ian else {
            panic!("expected InvalidAssetName");
        };
        assert_eq!(name, "../../shadow");
    }

    // `Unauthorized` carries no chained source (field-only struct variant, no boxed inner error).
    // The spec's source() table lists it under variants that return `None`.
    #[test]
    fn unauthorized_source_is_none() {
        assert!(
            Error::Unauthorized {
                status: 401,
                url: "https://example.com/api".to_string(),
            }
            .source()
            .is_none(),
            "Unauthorized must not expose a chained source()"
        );
        assert!(
            Error::Unauthorized {
                status: 403,
                url: "https://example.com/api".to_string(),
            }
            .source()
            .is_none(),
            "Unauthorized (403) must not expose a chained source()"
        );
    }

    // `HttpStatus` carries no chained source (field-only struct variant, no boxed inner error).
    // The spec's source() table lists it under variants that return `None`.
    #[test]
    fn http_status_variant_source_is_none() {
        assert!(
            Error::HttpStatus {
                status: 503,
                url: "https://example.com/releases".to_string(),
            }
            .source()
            .is_none(),
            "HttpStatus must not expose a chained source()"
        );
    }

    // `InvalidAssetName` Display: exact string with Debug-quoted name.
    // The Display format uses `{:?}` on the name, which wraps it in double-quotes.
    // This pins the full format, not just the prefix (unlike the update.rs version which only
    // asserts the prefix and embedded substring).
    #[test]
    fn invalid_asset_name_display_exact_string() {
        let err = Error::InvalidAssetName {
            name: "../etc/passwd".to_string(),
        };
        assert_eq!(
            err.to_string(),
            r#"InvalidAssetNameError: unsafe asset name: "../etc/passwd""#,
            "InvalidAssetName Display must match the spec string exactly"
        );
    }

    // `InvalidAssetName` carries no chained source (field-only struct variant).
    // The spec's source() table lists it under variants that return `None`.
    #[test]
    fn invalid_asset_name_source_is_none() {
        assert!(
            Error::InvalidAssetName {
                name: "../evil".to_string(),
            }
            .source()
            .is_none(),
            "InvalidAssetName must not expose a chained source()"
        );
    }

    // every variant has a non-panicking Display that keeps a sensible prefix and embeds its data.
    // The per-variant tests above cover the exact strings; this is a belt-and-suspenders sweep
    // that no variant lost its message or panics on Display.
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
            (
                Error::ArchiveVerificationRejected { reason: None },
                "ArchiveVerificationRejectedError:",
            ),
            (
                Error::ChecksumSourceInvalid {
                    asset: "a.tar.gz".into(),
                    reason: "no entry".into(),
                },
                "ChecksumSourceInvalidError:",
            ),
            (Error::NoReleaseFound { target: None }, "ReleaseError:"),
            (Error::missing_asset_field("f"), "ReleaseError:"),
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

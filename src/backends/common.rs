/*!
Configuration shared by every backend's `Update` builder.

Each backend (`github`, `gitlab`, `gitea`, `s3`) layers a small amount of
backend-specific configuration (repo coordinates, host/url, bucket, credentials) on top of
an identical set of common update options (target, bin name, version, progress style, …).

[`CommonBuilderConfig`] holds those common options while a backend builder is being
configured; [`CommonBuilderConfig::build`] validates them and produces a resolved
[`CommonConfig`] that each backend's `Update` embeds. The shared builder *setters* are
emitted into each backend builder by the `impl_common_builder_setters!` macro, and the
shared [`UpdateConfig`](crate::UpdateConfig) *accessors* are emitted as a full `impl` block for
each backend's `Update` by the `impl_update_config_accessors!` macro (both in `src/macros.rs`), so
the common surface lives in exactly one place.
*/

use std::path::PathBuf;
use std::time::Duration;

use crate::errors::*;
use crate::get_target;
use crate::http_client::HeaderMap;
#[cfg(feature = "progress-bar")]
use crate::{DEFAULT_PROGRESS_CHARS, DEFAULT_PROGRESS_TEMPLATE};

/// Per-request transport options shared by all of a backend's HTTP requests.
///
/// `headers` are extra headers merged into every request (on top of the backend's own auth /
/// user-agent headers); `timeout` bounds each request.
#[derive(Clone, Default)]
pub(crate) struct RequestConfig {
    pub(crate) timeout: Option<Duration>,
    pub(crate) headers: HeaderMap,
    /// Number of times to retry a failed API request (with exponential backoff).
    pub(crate) retries: u32,
    /// Optional user-supplied HTTP client to use through the [`HttpClient`](crate::http_client::HttpClient)
    /// trait instead of the per-call one the crate builds. `Arc`-backed so cloning a `RequestConfig`
    /// shares the client (and its connection pool).
    pub(crate) client: Option<std::sync::Arc<dyn crate::http_client::HttpClient>>,
    /// Optional user-supplied async HTTP client, mirroring [`client`](Self::client) for the async
    /// path. Async is reqwest-only.
    #[cfg(feature = "async")]
    pub(crate) async_client: Option<std::sync::Arc<dyn crate::http_client::AsyncHttpClient>>,
    /// First error produced converting a `request_header(name, value)` argument that wasn't a
    /// valid HTTP header. Stored here so the builder setter can stay infallible (`-> &mut Self`)
    /// and the failure is surfaced from `build()` as an `Error::InvalidHeader` instead of panicking.
    pub(crate) header_error: Option<String>,
}

impl std::fmt::Debug for RequestConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("RequestConfig");
        s.field("timeout", &self.timeout)
            .field("headers", &self.headers)
            .field("retries", &self.retries)
            .field("client", &self.client.as_ref().map(|_| "<http_client>"));
        #[cfg(feature = "async")]
        s.field(
            "async_client",
            &self.async_client.as_ref().map(|_| "<async_http_client>"),
        );
        s.field("header_error", &self.header_error).finish()
    }
}

impl RequestConfig {
    /// Insert an extra request header from `TryInto<HeaderName>` / `TryInto<HeaderValue>` args. A
    /// conversion failure is recorded in [`header_error`](Self::header_error) (first one wins) and
    /// surfaced later by [`check`](Self::check); the header is simply not inserted.
    pub(crate) fn insert_header<N, V>(&mut self, name: N, value: V)
    where
        N: ::core::convert::TryInto<crate::http_client::header::HeaderName>,
        V: ::core::convert::TryInto<crate::http_client::header::HeaderValue>,
    {
        let name = match name.try_into() {
            Ok(n) => n,
            Err(_) => {
                if self.header_error.is_none() {
                    self.header_error =
                        Some("invalid HTTP header name passed to `request_header`".to_string());
                }
                return;
            }
        };
        let value = match value.try_into() {
            Ok(v) => v,
            Err(_) => {
                if self.header_error.is_none() {
                    self.header_error =
                        Some("invalid HTTP header value passed to `request_header`".to_string());
                }
                return;
            }
        };
        self.headers.insert(name, value);
    }

    /// Return the stored `request_header` conversion error, if any, as an `Error::InvalidHeader`.
    pub(crate) fn check(&self) -> Result<()> {
        match &self.header_error {
            Some(msg) => Err(Error::InvalidHeader {
                source: Box::new(crate::errors::MessageError(msg.clone())),
            }),
            None => Ok(()),
        }
    }
}

/// The common, backend-independent options of an `Update` builder, before validation.
#[derive(Clone, Debug)]
pub(crate) struct CommonBuilderConfig {
    pub request: RequestConfig,
    pub target: Option<String>,
    pub asset_identifier: Option<String>,
    pub bin_name: Option<String>,
    pub bin_install_path: Option<PathBuf>,
    pub bin_path_in_archive: Option<String>,
    /// `true` when `bin_path_in_archive` was auto-derived from `bin_name` (not set explicitly by
    /// the user). Used by `bin_name` to re-derive when called again, while leaving an explicitly
    /// set value untouched.
    pub(crate) bin_path_in_archive_auto: bool,
    pub show_download_progress: bool,
    pub show_output: bool,
    pub no_confirm: bool,
    pub current_version: Option<String>,
    pub release_tag: Option<String>,
    #[cfg(feature = "progress-bar")]
    pub progress_template: String,
    #[cfg(feature = "progress-bar")]
    pub progress_chars: String,
    pub auth_token: Option<String>,
    pub progress_callback: Option<crate::ProgressCallback>,
    pub verify: Option<crate::VerifyCallback>,
    pub asset_matcher: Option<crate::AssetMatcher>,
    #[cfg(feature = "checksums")]
    pub checksum: Option<crate::Checksum>,
    #[cfg(feature = "signatures")]
    pub verifying_keys: Vec<[u8; zipsign_api::PUBLIC_KEY_LENGTH]>,
}

impl Default for CommonBuilderConfig {
    fn default() -> Self {
        Self {
            request: RequestConfig::default(),
            target: None,
            asset_identifier: None,
            bin_name: None,
            bin_install_path: None,
            bin_path_in_archive: None,
            bin_path_in_archive_auto: false,
            show_download_progress: false,
            show_output: true,
            no_confirm: false,
            current_version: None,
            release_tag: None,
            #[cfg(feature = "progress-bar")]
            progress_template: DEFAULT_PROGRESS_TEMPLATE.to_string(),
            #[cfg(feature = "progress-bar")]
            progress_chars: DEFAULT_PROGRESS_CHARS.to_string(),
            auth_token: None,
            progress_callback: None,
            verify: None,
            asset_matcher: None,
            #[cfg(feature = "checksums")]
            checksum: None,
            #[cfg(feature = "signatures")]
            verifying_keys: vec![],
        }
    }
}

impl CommonBuilderConfig {
    /// Validate the common options and resolve defaults, producing a [`CommonConfig`].
    ///
    /// `target` defaults to the crate's build target; `bin_install_path` defaults to the
    /// current executable. `current_version`, `bin_name`, and `bin_path_in_archive` are
    /// required (the last is set automatically by the `bin_name` setter).
    pub(crate) fn build(&self) -> Result<CommonConfig> {
        // Surface any deferred `request_header` conversion error as a config error.
        self.request.check()?;
        Ok(CommonConfig {
            request: self.request.clone(),
            target: self
                .target
                .clone()
                .unwrap_or_else(|| get_target().to_owned()),
            asset_identifier: self.asset_identifier.clone(),
            current_version: self.current_version.clone().ok_or(Error::MissingField {
                field: "current_version",
            })?,
            release_tag: self.release_tag.clone(),
            bin_name: self
                .bin_name
                .clone()
                .ok_or(Error::MissingField { field: "bin_name" })?,
            bin_install_path: match &self.bin_install_path {
                Some(p) => p.clone(),
                None => std::env::current_exe()?,
            },
            bin_path_in_archive: self
                .bin_path_in_archive
                .clone()
                .ok_or(Error::MissingField {
                    field: "bin_path_in_archive",
                })?,
            show_download_progress: self.show_download_progress,
            show_output: self.show_output,
            no_confirm: self.no_confirm,
            #[cfg(feature = "progress-bar")]
            progress_template: self.progress_template.clone(),
            #[cfg(feature = "progress-bar")]
            progress_chars: self.progress_chars.clone(),
            auth_token: self.auth_token.clone(),
            progress_callback: self.progress_callback.clone(),
            verify: self.verify.clone(),
            asset_matcher: self.asset_matcher.clone(),
            #[cfg(feature = "checksums")]
            checksum: self.checksum.clone(),
            #[cfg(feature = "signatures")]
            verifying_keys: self.verifying_keys.clone(),
        })
    }
}

/// The resolved common options of a built `Update`, embedded by every backend's `Update`.
#[derive(Debug)]
pub(crate) struct CommonConfig {
    pub request: RequestConfig,
    pub target: String,
    pub asset_identifier: Option<String>,
    pub current_version: String,
    pub release_tag: Option<String>,
    pub bin_name: String,
    pub bin_install_path: PathBuf,
    pub bin_path_in_archive: String,
    pub show_download_progress: bool,
    pub show_output: bool,
    pub no_confirm: bool,
    #[cfg(feature = "progress-bar")]
    pub progress_template: String,
    #[cfg(feature = "progress-bar")]
    pub progress_chars: String,
    pub auth_token: Option<String>,
    pub progress_callback: Option<crate::ProgressCallback>,
    pub verify: Option<crate::VerifyCallback>,
    pub asset_matcher: Option<crate::AssetMatcher>,
    #[cfg(feature = "checksums")]
    pub checksum: Option<crate::Checksum>,
    #[cfg(feature = "signatures")]
    pub verifying_keys: Vec<[u8; zipsign_api::PUBLIC_KEY_LENGTH]>,
}

#[cfg(test)]
mod tests {
    use super::{CommonBuilderConfig, RequestConfig};

    #[test]
    fn insert_header_records_invalid_value_error() {
        // The setter is infallible; an invalid *value* (control char) is deferred to `check()`
        // as an `Error::InvalidHeader` and the header is not inserted. (Only the invalid-*name* path
        // was tested at the backend level before; this covers the value branch directly.)
        let mut req = RequestConfig::default();
        req.insert_header("x-ok", "bad\nvalue");
        assert!(
            req.headers.get("x-ok").is_none(),
            "an invalid value must not be inserted"
        );
        let err = req
            .check()
            .expect_err("invalid value must surface from check()");
        match err {
            crate::errors::Error::InvalidHeader { source } => {
                assert!(
                    source.to_string().contains("value"),
                    "value-conversion error should mention the value, got: {}",
                    source
                );
            }
            other => panic!("expected Error::InvalidHeader, got {:?}", other),
        }
    }

    #[test]
    fn insert_header_records_invalid_name_error() {
        let mut req = RequestConfig::default();
        req.insert_header("inva lid", "ok");
        assert!(req.headers.get("inva lid").is_none());
        match req
            .check()
            .expect_err("invalid name must surface from check()")
        {
            crate::errors::Error::InvalidHeader { source } => {
                assert!(source.to_string().contains("name"))
            }
            other => panic!("expected Error::InvalidHeader, got {:?}", other),
        }
    }

    #[test]
    fn insert_header_first_error_wins() {
        // First a bad *name*, then a bad *value*. The recorded error must be the first one (name),
        // proving the `header_error.is_none()` guard keeps the earliest failure.
        let mut req = RequestConfig::default();
        req.insert_header("bad name", "ok"); // invalid name -> records "name" error
        req.insert_header("x-ok", "bad\nvalue"); // invalid value -> must NOT overwrite
        match req.check().expect_err("an error is recorded") {
            crate::errors::Error::InvalidHeader { source } => assert!(
                source.to_string().contains("name"),
                "the first (name) error must win, got: {}",
                source
            ),
            other => panic!("expected Error::InvalidHeader, got {:?}", other),
        }
    }

    #[test]
    fn insert_header_valid_then_invalid_still_keeps_valid_header() {
        // A valid header is inserted; a later invalid one is recorded as an error but does not
        // remove the already-inserted valid header.
        let mut req = RequestConfig::default();
        req.insert_header("x-good", "value");
        req.insert_header("x-bad", "bad\nvalue");
        assert_eq!(req.headers.get("x-good").unwrap(), "value");
        assert!(req.check().is_err());
    }

    #[test]
    fn check_is_ok_when_no_error_recorded() {
        let mut req = RequestConfig::default();
        req.insert_header("x-fine", "ok");
        assert!(req.check().is_ok());
        assert_eq!(req.headers.get("x-fine").unwrap(), "ok");
    }

    #[test]
    fn build_requires_current_version_bin_name_and_archive_path() {
        // Nothing set -> `current_version` missing.
        assert!(CommonBuilderConfig::default().build().is_err());

        // `current_version` set, but `bin_name` / `bin_path_in_archive` still missing.
        let cfg = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            ..Default::default()
        };
        assert!(cfg.build().is_err());

        // All required fields present.
        let cfg = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            bin_path_in_archive: Some("app".to_string()),
            ..Default::default()
        };
        let built = cfg.build().expect("all required fields present");
        assert_eq!(built.current_version, "0.1.0");
        assert_eq!(built.bin_name, "app");
    }

    #[test]
    fn build_resolves_target_and_install_path_defaults() {
        let base = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            bin_path_in_archive: Some("app".to_string()),
            ..Default::default()
        };

        // `target` unset -> defaults to the crate build target; install path -> current exe.
        let built = base.clone().build().unwrap();
        assert_eq!(built.target.as_str(), crate::get_target());
        assert!(!built.bin_install_path.as_os_str().is_empty());

        // `target` set -> used verbatim.
        let with_target = CommonBuilderConfig {
            target: Some("custom-target".to_string()),
            ..base
        };
        assert_eq!(with_target.build().unwrap().target, "custom-target");
    }

    // --- Item 5: self-fixing error messages --------------------------------------------------

    #[test]
    fn build_error_message_names_the_setter_for_current_version() {
        let err = CommonBuilderConfig::default().build().unwrap_err();
        match err {
            crate::errors::Error::MissingField { field } => {
                assert_eq!(
                    field, "current_version",
                    "the missing-field error must name `current_version`, got: {}",
                    field
                );
            }
            other => panic!("expected Error::MissingField, got {:?}", other),
        }
    }

    #[test]
    fn build_error_message_names_the_setter_for_bin_name() {
        let err = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            ..Default::default()
        }
        .build()
        .unwrap_err();
        match err {
            crate::errors::Error::MissingField { field } => {
                assert_eq!(
                    field, "bin_name",
                    "the missing-field error must name `bin_name`, got: {}",
                    field
                );
            }
            other => panic!("expected Error::MissingField, got {:?}", other),
        }
    }
}

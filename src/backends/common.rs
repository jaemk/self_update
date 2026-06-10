/*!
Configuration shared by every backend's `Update` builder.

Each backend (`github`, `gitlab`, `gitea`, `s3`) layers a small amount of
backend-specific configuration (repo coordinates, host/url, bucket, credentials) on top of
an identical set of common update options (target, bin name, version, progress style, …).

[`CommonBuilderConfig`] holds those common options while a backend builder is being
configured; [`CommonBuilderConfig::build`] validates them and produces a resolved
[`CommonConfig`] that each backend's `Update` embeds. The shared builder *setters* are
emitted into each backend builder by the `impl_common_builder_setters!` macro, and the
shared `ReleaseUpdate` *accessors* are emitted into each backend's `impl` by the
`impl_release_update_accessors!` macro (both in `src/macros.rs`), so the common surface
lives in exactly one place.
*/

use std::path::PathBuf;
use std::time::Duration;

use crate::errors::*;
use crate::http_client::HeaderMap;
use crate::{get_target, DEFAULT_PROGRESS_CHARS, DEFAULT_PROGRESS_TEMPLATE};

/// Per-request transport options shared by all of a backend's HTTP requests.
///
/// `headers` are extra headers merged into every request (on top of the backend's own auth /
/// user-agent headers); `timeout` bounds each request.
#[derive(Clone, Debug, Default)]
pub(crate) struct RequestConfig {
    pub(crate) timeout: Option<Duration>,
    pub(crate) headers: HeaderMap,
    /// Number of times to retry a failed listing request (with exponential backoff).
    pub(crate) retries: u32,
    /// Optional user-supplied HTTP client to use instead of the per-call one the crate builds.
    pub(crate) client: crate::http_client::ClientOverride,
}

/// The common, backend-independent options of an `Update` builder, before validation.
#[derive(Clone, Debug)]
pub(crate) struct CommonBuilderConfig {
    pub request: RequestConfig,
    pub target: Option<String>,
    pub identifier: Option<String>,
    pub bin_name: Option<String>,
    pub bin_install_path: Option<PathBuf>,
    pub bin_path_in_archive: Option<String>,
    pub show_download_progress: bool,
    pub show_output: bool,
    pub no_confirm: bool,
    pub current_version: Option<String>,
    pub target_version: Option<String>,
    pub progress_template: String,
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
            identifier: None,
            bin_name: None,
            bin_install_path: None,
            bin_path_in_archive: None,
            show_download_progress: false,
            show_output: true,
            no_confirm: false,
            current_version: None,
            target_version: None,
            progress_template: DEFAULT_PROGRESS_TEMPLATE.to_string(),
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
        Ok(CommonConfig {
            request: self.request.clone(),
            target: self
                .target
                .clone()
                .unwrap_or_else(|| get_target().to_owned()),
            identifier: self.identifier.clone(),
            current_version: self
                .current_version
                .clone()
                .ok_or_else(|| Error::Config("`current_version` required".to_string()))?,
            target_version: self.target_version.clone(),
            bin_name: self
                .bin_name
                .clone()
                .ok_or_else(|| Error::Config("`bin_name` required".to_string()))?,
            bin_install_path: match &self.bin_install_path {
                Some(p) => p.clone(),
                None => std::env::current_exe()?,
            },
            bin_path_in_archive: self
                .bin_path_in_archive
                .clone()
                .ok_or_else(|| Error::Config("`bin_path_in_archive` required".to_string()))?,
            show_download_progress: self.show_download_progress,
            show_output: self.show_output,
            no_confirm: self.no_confirm,
            progress_template: self.progress_template.clone(),
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
    pub identifier: Option<String>,
    pub current_version: String,
    pub target_version: Option<String>,
    pub bin_name: String,
    pub bin_install_path: PathBuf,
    pub bin_path_in_archive: String,
    pub show_download_progress: bool,
    pub show_output: bool,
    pub no_confirm: bool,
    pub progress_template: String,
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
    use super::CommonBuilderConfig;

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
}

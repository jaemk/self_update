/*!
Configuration shared by every backend's `Update` builder.

Each backend (`github`, `gitlab`, `gitea`, `s3`, `custom`) layers a small amount of
backend-specific configuration (repo coordinates, host/url, bucket, credentials, a release
source) on top of an identical set of common update options (target, bin name, version,
progress style, …).

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
use crate::http_client::header;

/// The HTTP authorization scheme a backend uses to present its auth token.
///
/// The token is rendered into the `Authorization` header as `"<scheme> <token>"`: `token <token>`
/// for [`Token`](AuthScheme::Token) (github/gitea) and `Bearer <token>` for
/// [`Bearer`](AuthScheme::Bearer) (gitlab). The scheme is a per-backend default carried in
/// [`RequestConfig`]; it is applied by the shared header-derivation
/// ([`RequestConfig::apply_auth`]) on both the listing and the download paths, and is overridden
/// when the user sets their own `Authorization` via `request_header`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AuthScheme {
    /// `Authorization: token <token>` (github, gitea).
    #[default]
    Token,
    /// `Authorization: Bearer <token>` (gitlab). Only constructed when the `gitlab` backend is
    /// enabled; the `allow(dead_code)` keeps it from warning in builds without that backend.
    #[cfg_attr(not(feature = "gitlab"), allow(dead_code))]
    Bearer,
}

impl AuthScheme {
    /// The header-value prefix this scheme renders before the token (`"token"` / `"Bearer"`).
    fn prefix(self) -> &'static str {
        match self {
            AuthScheme::Token => "token",
            AuthScheme::Bearer => "Bearer",
        }
    }
}

/// The boxed inner error of an [`Error::SemVer`] produced from a server-supplied release tag:
/// names the offending tag in its message and keeps the original `semver` parse failure
/// reachable via [`std::error::Error::source`].
#[cfg_attr(
    not(any(feature = "github", feature = "gitlab", feature = "gitea")),
    allow(dead_code)
)]
#[derive(Debug)]
pub(crate) struct NonSemverTagError {
    tag: String,
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl std::fmt::Display for NonSemverTagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "release tag `{}` is not a semver version: {}",
            self.tag, self.source
        )
    }
}

impl std::error::Error for NonSemverTagError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

/// Rewrap a semver-validation failure from `Release::builder().build()` so the error names the
/// offending release tag (`nightly`, `latest`, a date, ...) instead of surfacing a bare parse
/// failure with no context. The original parse error stays on the `source()` chain. Non-`SemVer`
/// errors pass through unchanged.
///
/// Only the forge backends (github/gitlab/gitea) funnel server-supplied tags through the builder;
/// the attribute keeps builds without any of them warning-free.
#[cfg_attr(
    not(any(feature = "github", feature = "gitlab", feature = "gitea")),
    allow(dead_code)
)]
pub(crate) fn name_tag_in_semver_error(tag: &str, err: Error) -> Error {
    match err {
        Error::SemVer(inner) => Error::SemVer(Box::new(NonSemverTagError {
            tag: tag.to_owned(),
            source: inner,
        })),
        other => other,
    }
}

/// Strip the configured tag prefix (or the conventional leading `v`) from a release tag to get the
/// bare version candidate.
///
/// With `prefix = None`, a single leading lowercase `v` is trimmed (the long-standing default) and
/// the result is always `Some`. With `prefix = Some(p)`, the tag must start with `p`; `p` is
/// stripped and a leading `v` after it is also trimmed (so `myapp-v1.2.3` and `myapp-1.2.3` both
/// yield `Some("1.2.3")`). A tag that does not start with `p` yields `None`, so the caller skips it:
/// with a prefix configured, only tags carrying it count as releases (a bare `1.0.0` tag is not
/// silently accepted just because its remainder parses as semver).
#[cfg_attr(
    not(any(feature = "github", feature = "gitlab", feature = "gitea")),
    allow(dead_code)
)]
pub(crate) fn strip_tag_prefix(tag: &str, prefix: Option<&str>) -> Option<String> {
    match prefix {
        None => Some(tag.trim_start_matches('v').to_owned()),
        Some(p) => tag
            .strip_prefix(p)
            .map(|rest| rest.trim_start_matches('v').to_owned()),
    }
}

/// Build the skippable [`Error::SemVer`] returned when a tag does not carry the configured
/// `tag_prefix`. It uses the `SemVer` variant so the forge listing walk drops the release (its skip
/// arm keys on `Error::SemVer`), the same way it drops a non-semver tag.
#[cfg_attr(
    not(any(feature = "github", feature = "gitlab", feature = "gitea")),
    allow(dead_code)
)]
pub(crate) fn tag_prefix_mismatch_error(tag: &str, prefix: &str) -> Error {
    Error::SemVer(Box::new(crate::errors::MessageError(format!(
        "release tag `{tag}` does not start with the configured tag_prefix `{prefix}`"
    ))))
}

/// The lowercased host of a URL, for auth-origin comparison. Parses with `http::Uri` (always
/// available, no `url` crate needed). Returns `None` when the URL has no host.
#[cfg_attr(
    not(any(feature = "github", feature = "gitlab", feature = "gitea")),
    allow(dead_code)
)]
pub(crate) fn host_of(url: &str) -> Option<String> {
    url.parse::<http::Uri>().ok()?.host().map(|h| {
        h.trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase()
    })
}
#[cfg(feature = "progress-bar")]
use crate::{DEFAULT_PROGRESS_CHARS, DEFAULT_PROGRESS_TEMPLATE};

/// Per-request transport options shared by all of a backend's HTTP requests.
///
/// `headers` are extra headers merged into every request (on top of the backend's own auth /
/// user-agent headers); `timeout` bounds each request.
#[derive(Clone)]
pub(crate) struct RequestConfig {
    pub(crate) timeout: Option<Duration>,
    pub(crate) headers: HeaderMap,
    /// Number of times to retry a failed API request (with exponential backoff).
    pub(crate) retries: u32,
    /// Base delay (attempt 0) for the exponential retry backoff. The delay doubles each attempt up
    /// to [`retry_max_delay`](Self::retry_max_delay). Defaults to 100ms.
    pub(crate) retry_base_delay: Duration,
    /// Cap on the exponential retry backoff delay. Defaults to ~3.2s (100ms << 5).
    pub(crate) retry_max_delay: Duration,
    /// The backend's authorization scheme for rendering [`auth_token`](Self::auth_token) into the
    /// `Authorization` header. Per-backend default (github/gitea `Token`, gitlab `Bearer`).
    pub(crate) auth_scheme: AuthScheme,
    /// The backend auth token, if any, rendered via [`auth_scheme`](Self::auth_scheme). A user
    /// `request_header(AUTHORIZATION, ..)` override in [`headers`](Self::headers) takes precedence.
    pub(crate) auth_token: Option<String>,
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
    /// Custom TLS root CA certificates to bake into the HTTP client the crate builds when no client
    /// was injected. Materialized into [`client`](Self::client) (and [`async_client`](Self::async_client))
    /// by [`build_client`](Self::build_client) at `build()` time.
    pub(crate) root_certificates: Vec<crate::tls::Certificate>,
    /// First error produced materializing a client from [`root_certificates`](Self::root_certificates)
    /// (invalid cert bytes or a client-build failure). Deferred like [`header_error`](Self::header_error)
    /// and surfaced from [`check`](Self::check) as an `Error::InvalidCertificate`.
    pub(crate) cert_error: Option<String>,
    /// The host of the backend's configured API base (e.g. `api.github.com`, `gitlab.com`, the
    /// gitea host). The derived [`auth_token`](Self::auth_token) is only attached to a request whose
    /// host matches this (or an [`auth_hosts`](Self::auth_hosts) entry), so a server-supplied asset
    /// `download_url` or `Link` next-page URL pointing at a different host does not receive the
    /// token. Set by each backend at `build()` time; `None` disables the token entirely.
    pub(crate) auth_base_host: Option<String>,
    /// Additional hosts the user has explicitly authorized to receive the auth token, via
    /// `allow_auth_host`. Checked alongside [`auth_base_host`](Self::auth_base_host).
    pub(crate) auth_hosts: Vec<String>,
    /// When `true`, the auth token may be attached over plain `http` (not just `https`) to a
    /// host-matched request. Off by default; set via `dangerously_allow_non_https_auth_forwarding`.
    pub(crate) allow_insecure_auth: bool,
}

/// Default base delay for the exponential retry backoff (attempt 0).
pub(crate) const DEFAULT_RETRY_BASE_DELAY: Duration = Duration::from_millis(100);
/// Default cap on the exponential retry backoff (100ms << 5 == 3200ms).
pub(crate) const DEFAULT_RETRY_MAX_DELAY: Duration = Duration::from_millis(3200);

impl Default for RequestConfig {
    fn default() -> Self {
        Self {
            timeout: None,
            headers: HeaderMap::new(),
            retries: 0,
            retry_base_delay: DEFAULT_RETRY_BASE_DELAY,
            retry_max_delay: DEFAULT_RETRY_MAX_DELAY,
            auth_scheme: AuthScheme::default(),
            auth_token: None,
            client: None,
            #[cfg(feature = "async")]
            async_client: None,
            header_error: None,
            root_certificates: Vec::new(),
            cert_error: None,
            auth_base_host: None,
            auth_hosts: Vec::new(),
            allow_insecure_auth: false,
        }
    }
}

impl std::fmt::Debug for RequestConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("RequestConfig");
        s.field("timeout", &self.timeout)
            .field("headers", &self.headers)
            .field("retries", &self.retries)
            .field("retry_base_delay", &self.retry_base_delay)
            .field("retry_max_delay", &self.retry_max_delay)
            .field("auth_scheme", &self.auth_scheme)
            .field("auth_token", &self.auth_token.as_ref().map(|_| "<token>"))
            .field("client", &self.client.as_ref().map(|_| "<http_client>"));
        #[cfg(feature = "async")]
        s.field(
            "async_client",
            &self.async_client.as_ref().map(|_| "<async_http_client>"),
        );
        s.field("header_error", &self.header_error)
            .field(
                "root_certificates",
                &format_args!("<{} root_certificates>", self.root_certificates.len()),
            )
            .field("cert_error", &self.cert_error)
            .finish()
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

    /// Apply this config's derived authorization to `headers`, honoring a user override.
    ///
    /// This is the single header-derivation used by **both** the listing path
    /// ([`send`](crate::backends::send) / `send_async`) and the download path
    /// ([`build_download`](crate::update)). Precedence:
    ///
    /// 1. If the user supplied their own `Authorization` via `request_header` (present in
    ///    [`headers`](Self::headers)), it wins and the backend scheme/token are not applied.
    /// 2. Otherwise, if an [`auth_token`](Self::auth_token) is set, it is rendered as
    ///    `"<scheme> <token>"` per [`auth_scheme`](Self::auth_scheme) and inserted.
    /// 3. Otherwise nothing is inserted.
    ///
    /// A token that does not encode as a header value surfaces as
    /// [`Error::InvalidAuthToken`](crate::errors::Error::InvalidAuthToken).
    ///
    /// The token is attached only when [`auth_allowed_for`](Self::auth_allowed_for) permits it for
    /// `url` (same host as the configured API base or an `allow_auth_host` entry, over https).
    /// A server-supplied asset `download_url` or `Link` next-page URL pointing at a different host
    /// gets no token, so a malicious release server cannot harvest the credential.
    pub(crate) fn apply_auth(&self, url: &str, headers: &mut HeaderMap) -> Result<()> {
        // A user-supplied Authorization header (via `request_header`) always wins.
        if self.headers.contains_key(header::AUTHORIZATION) {
            return Ok(());
        }
        let Some(token) = self.auth_token.as_deref() else {
            return Ok(());
        };
        if !self.auth_allowed_for(url) {
            log::warn!(
                "self_update: not attaching the auth token to {url}: its host is not the configured \
                 API host and is not in the allow_auth_host set (or the scheme is not https). The \
                 request proceeds without authorization."
            );
            return Ok(());
        }
        let mut value = format!("{} {}", self.auth_scheme.prefix(), token)
            .parse::<header::HeaderValue>()
            .map_err(|err| Error::InvalidAuthToken {
                source: Box::new(err),
            })?;
        // Mark the value sensitive so it renders as `Sensitive` in any `Debug` (e.g. a `Download`'s)
        // and is kept out of logs by the HTTP client.
        value.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, value);
        Ok(())
    }

    /// Whether the derived auth token may be attached to a request to `url`.
    ///
    /// The host must match the configured [`auth_base_host`](Self::auth_base_host) or an
    /// [`auth_hosts`](Self::auth_hosts) entry, and the scheme must be `https` -- except for loopback
    /// hosts (`localhost`, `127.0.0.1`, `::1`), which are allowed over plain http so a local mirror
    /// and the loopback test stubs keep working.
    pub(crate) fn auth_allowed_for(&self, url: &str) -> bool {
        let uri = match url.parse::<http::Uri>() {
            Ok(u) => u,
            Err(_) => return false,
        };
        let host = match uri.host() {
            Some(h) => h
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_ascii_lowercase(),
            None => return false,
        };
        let host_matches = self
            .auth_base_host
            .as_deref()
            .is_some_and(|b| b.eq_ignore_ascii_case(&host))
            || self
                .auth_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(&host));
        if !host_matches {
            return false;
        }
        let is_loopback = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(false);
        uri.scheme_str() == Some("https") || is_loopback || self.allow_insecure_auth
    }

    /// Materialize a pre-configured HTTP client from `root_certificates` if set and no client was
    /// injected. On success, stores the client in `self.client` (and the async sibling). On failure,
    /// records the error in `self.cert_error` (first error wins, mirroring `header_error`).
    ///
    /// Each client slot is materialized independently: the sync client is built from the certs only
    /// when the sync slot is empty, and the async client only when the async slot is empty. So
    /// injecting a client for one transport does not drop the custom roots for the other (the
    /// injected client owns its own TLS; the auto-built one still trusts the certs). A cert-build
    /// failure for a slot that will actually be built is recorded in `cert_error`.
    pub(crate) fn build_client(&mut self) {
        if self.root_certificates.is_empty() {
            return;
        }
        if self.client.is_none() {
            match crate::http_client::client_with_root_certs(&self.root_certificates) {
                Ok(c) => self.client = Some(c),
                Err(e) => {
                    if self.cert_error.is_none() {
                        self.cert_error = Some(e.to_string());
                    }
                }
            }
        }
        #[cfg(feature = "async")]
        if self.async_client.is_none() {
            match crate::http_client::async_client_with_root_certs(&self.root_certificates) {
                Ok(c) => self.async_client = Some(c),
                Err(e) => {
                    if self.cert_error.is_none() {
                        self.cert_error = Some(e.to_string());
                    }
                }
            }
        }
    }

    /// Surface any deferred config error: a `request_header` conversion failure as
    /// `Error::InvalidHeader` (checked first, so it takes precedence), then a root-certificate /
    /// client-build failure as `Error::InvalidCertificate`.
    pub(crate) fn check(&self) -> Result<()> {
        if let Some(msg) = &self.header_error {
            return Err(Error::InvalidHeader {
                source: Box::new(crate::errors::MessageError(msg.clone())),
            });
        }
        if let Some(msg) = &self.cert_error {
            return Err(Error::InvalidCertificate {
                source: Box::new(crate::errors::MessageError(msg.clone())),
            });
        }
        Ok(())
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
    pub show_release_notes: bool,
    pub update_strategy: crate::update::UpdateStrategy,
    /// Optional tag prefix used to derive the version from a release tag (e.g. `myapp-` for a
    /// monorepo tag `myapp-1.2.3`). `None` keeps the default of trimming a leading `v`. Only the
    /// forge backends (github/gitlab/gitea) consult it; set via their `tag_prefix` setter.
    pub tag_prefix: Option<String>,
    pub current_version: Option<String>,
    pub release_tag: Option<String>,
    #[cfg(feature = "progress-bar")]
    pub progress_template: String,
    #[cfg(feature = "progress-bar")]
    pub progress_chars: String,
    pub auth_token: Option<String>,
    /// The backend's authorization scheme. Defaults to [`AuthScheme::Token`] (github/gitea); gitlab
    /// sets [`AuthScheme::Bearer`]. Threaded into the resolved [`RequestConfig::auth_scheme`].
    pub auth_scheme: AuthScheme,
    pub progress_callback: Option<crate::ProgressCallback>,
    pub verify: Option<crate::VerifyCallback>,
    pub asset_matcher: Option<crate::AssetMatcher>,
    #[cfg(feature = "checksums")]
    pub checksum: Option<crate::Checksum>,
    /// Verify the download against the backend-published asset digest when one is present.
    /// On by default; `verify_release_digest(false)` opts out.
    #[cfg(feature = "checksums")]
    pub verify_release_digest: bool,
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
            show_release_notes: false,
            update_strategy: crate::update::UpdateStrategy::default(),
            tag_prefix: None,
            current_version: None,
            release_tag: None,
            #[cfg(feature = "progress-bar")]
            progress_template: DEFAULT_PROGRESS_TEMPLATE.to_string(),
            #[cfg(feature = "progress-bar")]
            progress_chars: DEFAULT_PROGRESS_CHARS.to_string(),
            auth_token: None,
            auth_scheme: AuthScheme::default(),
            progress_callback: None,
            verify: None,
            asset_matcher: None,
            #[cfg(feature = "checksums")]
            checksum: None,
            #[cfg(feature = "checksums")]
            verify_release_digest: true,
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
        // Resolve the auth scheme/token into the request config so the shared header-derivation
        // (`apply_auth`) can apply it on both the listing and download paths.
        let mut request = self.request.clone();
        request.auth_scheme = self.auth_scheme;
        request.auth_token = self.auth_token.clone();
        // Materialize an HTTP client from any custom root CA certs (no-op if none / a client was
        // injected), then surface any deferred header/cert error as a config error.
        request.build_client();
        request.check()?;
        Ok(CommonConfig {
            request,
            target: self
                .target
                .clone()
                .unwrap_or_else(|| get_target().to_owned()),
            asset_identifier: self.asset_identifier.clone(),
            current_version: self.current_version.clone().ok_or(Error::MissingField {
                field: "current_version",
            })?,
            release_tag: self.release_tag.clone(),
            tag_prefix: self.tag_prefix.clone(),
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
            show_release_notes: self.show_release_notes,
            update_strategy: self.update_strategy,
            #[cfg(feature = "progress-bar")]
            progress_template: self.progress_template.clone(),
            #[cfg(feature = "progress-bar")]
            progress_chars: self.progress_chars.clone(),
            progress_callback: self.progress_callback.clone(),
            verify: self.verify.clone(),
            asset_matcher: self.asset_matcher.clone(),
            #[cfg(feature = "checksums")]
            checksum: self.checksum.clone(),
            #[cfg(feature = "checksums")]
            verify_release_digest: self.verify_release_digest,
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
    pub tag_prefix: Option<String>,
    pub bin_name: String,
    pub bin_install_path: PathBuf,
    pub bin_path_in_archive: String,
    pub show_download_progress: bool,
    pub show_output: bool,
    pub no_confirm: bool,
    pub show_release_notes: bool,
    pub update_strategy: crate::update::UpdateStrategy,
    #[cfg(feature = "progress-bar")]
    pub progress_template: String,
    #[cfg(feature = "progress-bar")]
    pub progress_chars: String,
    pub progress_callback: Option<crate::ProgressCallback>,
    pub verify: Option<crate::VerifyCallback>,
    pub asset_matcher: Option<crate::AssetMatcher>,
    #[cfg(feature = "checksums")]
    pub checksum: Option<crate::Checksum>,
    #[cfg(feature = "checksums")]
    pub verify_release_digest: bool,
    #[cfg(feature = "signatures")]
    pub verifying_keys: Vec<[u8; zipsign_api::PUBLIC_KEY_LENGTH]>,
}

#[cfg(test)]
mod tests {
    use super::{CommonBuilderConfig, RequestConfig};

    /// A PEM-framed certificate whose body is not valid X.509 DER (base64 of "not a valid cert").
    /// reqwest accepts the PEM framing but rejects it at client-build time, so it reliably produces
    /// a cert-build error. Used by the per-slot cert tests, which are `async`-gated (async implies
    /// reqwest).
    #[cfg(feature = "async")]
    const BAD_PEM_CERT: &[u8] =
        b"-----BEGIN CERTIFICATE-----\nbm90IGEgdmFsaWQgY2VydA==\n-----END CERTIFICATE-----\n";

    // `name_tag_in_semver_error` names the tag in the message and keeps the original
    // `semver::Error` reachable through the `source()` chain (SemVer -> NonSemverTagError ->
    // semver::Error), so callers walking the chain still find the parse failure.
    #[test]
    fn name_tag_in_semver_error_names_tag_and_keeps_source_chain() {
        let parse_err = "nightly".parse::<semver::Version>().unwrap_err();
        let parse_msg = parse_err.to_string();
        let wrapped =
            super::name_tag_in_semver_error("nightly", crate::errors::Error::from(parse_err));
        let crate::errors::Error::SemVer(inner) = &wrapped else {
            panic!("expected Error::SemVer, got {wrapped:?}");
        };
        assert!(
            inner.to_string().contains("`nightly`"),
            "the message must name the tag, got: {inner}"
        );
        let chained = inner
            .source()
            .expect("the original semver parse error must stay on the chain");
        assert_eq!(chained.to_string(), parse_msg);
    }

    // Non-SemVer errors pass through `name_tag_in_semver_error` unchanged.
    #[test]
    fn name_tag_in_semver_error_passes_other_errors_through() {
        let err = crate::errors::Error::MissingField { field: "version" };
        let out = super::name_tag_in_semver_error("nightly", err);
        assert!(
            matches!(out, crate::errors::Error::MissingField { field: "version" }),
            "non-SemVer errors must pass through unchanged, got {out:?}"
        );
    }

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
    fn build_defaults_and_propagates_update_strategy() {
        // Default is `Compatible`; an explicit `Latest` is carried into the resolved config.
        let base = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            bin_path_in_archive: Some("app".to_string()),
            ..Default::default()
        };
        assert_eq!(
            base.clone().build().unwrap().update_strategy,
            crate::update::UpdateStrategy::Compatible,
            "the default update strategy must be Compatible"
        );

        let latest = CommonBuilderConfig {
            update_strategy: crate::update::UpdateStrategy::Latest,
            ..base
        };
        assert_eq!(
            latest.build().unwrap().update_strategy,
            crate::update::UpdateStrategy::Latest,
            "an explicit Latest strategy must be carried into the resolved config"
        );
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

    // --- CORP-1: custom root CA certificates ------------------------------------------------

    #[test]
    fn build_client_with_no_certs_leaves_client_none() {
        // With no `root_certificates`, `build_client` is a no-op: it must not attempt any client
        // construction and must leave `client` as `None` (the crate default path stays in effect).
        let mut req = RequestConfig::default();
        req.build_client();
        assert!(
            req.client.is_none(),
            "no certs => build_client must not materialize a client"
        );
        assert!(req.cert_error.is_none(), "no certs => no cert_error");
    }

    #[test]
    fn build_client_with_injected_client_skips_cert_build() {
        // An injected client wins: even with a (garbage) cert present, `build_client` must NOT try to
        // build a client over it, so no `cert_error` is recorded and the injected client is kept.
        struct DummyClient;
        impl crate::http_client::HttpClient for DummyClient {
            fn get(
                &self,
                _url: &str,
                _headers: &crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> crate::Result<Box<dyn crate::http_client::HttpResponse>> {
                unreachable!("not called in this test")
            }
        }
        let mut req = RequestConfig {
            client: Some(std::sync::Arc::new(DummyClient)),
            ..Default::default()
        };
        req.root_certificates
            .push(crate::tls::Certificate::from_pem(b"garbage".to_vec()));
        req.build_client();
        assert!(
            req.cert_error.is_none(),
            "an injected client must short-circuit the sync cert build (no cert_error)"
        );
        assert!(req.client.is_some(), "the injected client must be kept");
    }

    // Regression for H3: when a sync client is injected but the async slot is empty, and the cert
    // bytes are garbage, the async cert-build must NOT run and must NOT set cert_error. Previously
    // the async block ran independently of the sync guard, so a cert parse failure surfaced as a
    // cert_error even though the injected sync client was valid.
    #[cfg(feature = "async")]
    #[test]
    fn build_client_injected_sync_still_builds_async_from_certs() {
        // Per-slot cert materialization: injecting a sync client does NOT skip the async slot's
        // cert-build. The injected sync client is kept as-is, but the async client is still built
        // from the custom roots (so async listing trusts the CA). With garbage cert bytes the async
        // build fails, which is recorded in cert_error -- proving the async slot ran.
        struct DummyClient;
        impl crate::http_client::HttpClient for DummyClient {
            fn get(
                &self,
                _url: &str,
                _headers: &crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> crate::Result<Box<dyn crate::http_client::HttpResponse>> {
                unreachable!("not called in this test")
            }
        }
        let mut req = RequestConfig {
            client: Some(std::sync::Arc::new(DummyClient)),
            ..Default::default()
        };
        req.root_certificates
            .push(crate::tls::Certificate::from_pem(BAD_PEM_CERT.to_vec()));
        req.build_client();
        assert!(
            req.cert_error.is_some(),
            "the async slot must attempt the cert-build even when a sync client is injected"
        );
        assert!(
            req.client.is_some(),
            "the injected sync client must be kept as-is"
        );
    }

    // The bad-cert path only records an error when a real client backend can attempt (and reject)
    // the parse. With neither client feature, `client_with_root_certs` returns the
    // "no HTTP client feature enabled" error instead, which still populates `cert_error`.
    #[cfg(any(feature = "reqwest", feature = "ureq"))]
    #[test]
    fn build_client_bad_cert_records_cert_error() {
        // A malformed cert with no injected client: `build_client` asks the active backend to build
        // a client, the parse fails, and the error is recorded in `cert_error`. The two backends
        // reject different malformed inputs, so the bad bytes are selected to match the same backend
        // `client_with_root_certs` dispatches to (reqwest preferred when both features are on):
        //   - reqwest validates at client-build time, accepting PEM framing but rejecting a body that
        //     decodes to non-X.509-DER bytes (base64 of "not a valid cert").
        //   - ureq validates the PEM framing in `from_pem` (deferring DER), so it rejects bytes that
        //     contain no PEM certificate at all.
        #[cfg(feature = "reqwest")]
        let bad_cert = crate::tls::Certificate::from_pem(
            b"-----BEGIN CERTIFICATE-----\nbm90IGEgdmFsaWQgY2VydA==\n-----END CERTIFICATE-----\n"
                .to_vec(),
        );
        #[cfg(all(feature = "ureq", not(feature = "reqwest")))]
        let bad_cert = crate::tls::Certificate::from_pem(b"not a pem certificate".to_vec());

        let mut req = RequestConfig::default();
        req.root_certificates.push(bad_cert);
        req.build_client();
        assert!(
            req.cert_error.is_some(),
            "a malformed cert must record a cert_error"
        );
        assert!(
            req.client.is_none(),
            "a failed cert build must not leave a client"
        );
    }

    #[test]
    fn check_surfaces_cert_error_as_invalid_certificate() {
        // A recorded `cert_error` (and no header error) surfaces from `check()` as
        // `Error::InvalidCertificate` carrying the stored message via `source()`.
        let req = RequestConfig {
            cert_error: Some("boom".to_string()),
            ..Default::default()
        };
        match req
            .check()
            .expect_err("cert_error must surface from check()")
        {
            crate::errors::Error::InvalidCertificate { source } => {
                assert_eq!(source.to_string(), "boom")
            }
            other => panic!("expected Error::InvalidCertificate, got {:?}", other),
        }
    }

    #[test]
    fn check_surfaces_header_error_before_cert_error() {
        // When BOTH a header error and a cert error are present, `check()` must report the header
        // error (`Error::InvalidHeader`) first: header validation takes precedence.
        let req = RequestConfig {
            header_error: Some("bad header".to_string()),
            cert_error: Some("bad cert".to_string()),
            ..Default::default()
        };
        match req.check().expect_err("an error must surface") {
            crate::errors::Error::InvalidHeader { .. } => {}
            other => panic!("expected Error::InvalidHeader to win, got {:?}", other),
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

    #[test]
    fn build_error_message_names_the_setter_for_bin_path_in_archive() {
        // With current_version and bin_name both set, the only remaining required field is
        // bin_path_in_archive. Verify the error names that field specifically.
        let err = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            ..Default::default()
        }
        .build()
        .unwrap_err();
        match err {
            crate::errors::Error::MissingField { field } => {
                assert_eq!(
                    field, "bin_path_in_archive",
                    "the missing-field error must name `bin_path_in_archive`, got: {}",
                    field
                );
            }
            other => panic!("expected Error::MissingField, got {:?}", other),
        }
    }

    // --- apply_auth: auth-header derivation --------------------------------------------------

    #[test]
    fn apply_auth_no_token_is_noop() {
        // With no auth_token set, apply_auth must not insert any Authorization header.
        let req = RequestConfig::default();
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://api.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "apply_auth with no token must not insert an Authorization header"
        );
    }

    #[test]
    fn apply_auth_token_scheme_inserts_authorization_header() {
        // With auth_token set and the default Token scheme, apply_auth must insert
        // "Authorization: token <token>" for a request to the configured API host.
        let req = RequestConfig {
            auth_token: Some("mytoken".to_string()),
            auth_base_host: Some("api.example.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://api.example.com/x", &mut headers)
            .unwrap();
        let auth = headers
            .get(crate::http_client::header::AUTHORIZATION)
            .expect("apply_auth must insert an Authorization header when a token is set");
        assert_eq!(
            auth, "token mytoken",
            "Token scheme must render as 'token <token>'"
        );
    }

    #[test]
    fn apply_auth_user_supplied_authorization_header_wins() {
        // When the user sets their own Authorization header via `request_header`, apply_auth
        // must see it in self.headers and return early without inserting the crate's token into
        // the passed-in headers map. The crate must never overwrite the user's own auth.
        let mut req = RequestConfig {
            auth_token: Some("should-not-appear".to_string()),
            ..Default::default()
        };
        req.insert_header(
            crate::http_client::header::AUTHORIZATION,
            "custom my-custom-token",
        );
        let mut out_headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://api.example.com/x", &mut out_headers)
            .unwrap();
        assert!(
            out_headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "apply_auth must not insert its token when the user supplied their own Authorization"
        );
    }

    #[test]
    fn apply_auth_bearer_scheme_renders_bearer_prefix() {
        // The Bearer auth scheme (gitlab) must render as "Bearer <token>".
        // AuthScheme::Bearer is always compiled in (the allow(dead_code) attr only suppresses
        // the lint warning in non-gitlab builds), so this test is valid across all feature sets.
        let req = RequestConfig {
            auth_token: Some("mytoken".to_string()),
            auth_scheme: super::AuthScheme::Bearer,
            auth_base_host: Some("api.example.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://api.example.com/x", &mut headers)
            .unwrap();
        let auth = headers
            .get(crate::http_client::header::AUTHORIZATION)
            .expect("apply_auth must insert an Authorization header");
        assert_eq!(
            auth, "Bearer mytoken",
            "Bearer scheme must render as 'Bearer <token>'"
        );
    }

    #[test]
    fn apply_auth_invalid_token_surfaces_invalid_auth_token_error() {
        // A token that contains a control character (newline) cannot be encoded as an HTTP
        // header value. apply_auth must surface this as Error::InvalidAuthToken, not panic.
        let req = RequestConfig {
            auth_token: Some("bad\ntoken".to_string()),
            auth_base_host: Some("api.example.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        match req.apply_auth("https://api.example.com/x", &mut headers) {
            Err(crate::errors::Error::InvalidAuthToken { .. }) => {}
            other => panic!(
                "expected Error::InvalidAuthToken for a token with a newline, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn apply_auth_not_attached_to_cross_origin_url() {
        // The token must NOT be attached to a request whose host differs from the configured API
        // host. A malicious release server that sets the asset download_url (or a Link next-page
        // URL) to its own host must not receive the credential.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("api.github.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://evil.example.com/x.tar.gz", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "the token must not be attached to a cross-origin URL"
        );
    }

    #[test]
    fn apply_auth_not_attached_over_plaintext_http() {
        // The token must NOT be sent over plaintext http to a non-loopback host, even when the host
        // matches the configured API host (guards against a downgraded/misconfigured URL).
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("api.example.com".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("http://api.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "the token must not be sent over plaintext http to a non-loopback host"
        );
    }

    #[test]
    fn apply_auth_attached_to_allow_auth_host() {
        // A host the user explicitly authorized via allow_auth_host receives the token even though
        // it differs from the API base host.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("api.example.com".to_string()),
            auth_hosts: vec!["cdn.example.com".to_string()],
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("https://cdn.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_some(),
            "an allow_auth_host entry must receive the token"
        );
    }

    #[test]
    fn apply_auth_over_http_when_insecure_forwarding_allowed() {
        // With the escape hatch set, the token is attached over plain http to a host-matched
        // (non-loopback) request.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("internal.example.com".to_string()),
            allow_insecure_auth: true,
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("http://internal.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_some(),
            "the escape hatch must allow the token over http to a host-matched request"
        );
    }

    #[test]
    fn apply_auth_insecure_flag_still_requires_host_match() {
        // The escape hatch only lifts the https requirement; a cross-origin host still gets no token.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("internal.example.com".to_string()),
            allow_insecure_auth: true,
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("http://evil.example.com/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "the escape hatch must not attach the token to a cross-origin host"
        );
    }

    #[test]
    fn apply_auth_attached_to_loopback_over_http() {
        // Loopback hosts may use plain http (local mirrors and the loopback test stubs), provided
        // the host matches the configured base.
        let req = RequestConfig {
            auth_token: Some("secret".to_string()),
            auth_base_host: Some("127.0.0.1".to_string()),
            ..Default::default()
        };
        let mut headers = crate::http_client::HeaderMap::new();
        req.apply_auth("http://127.0.0.1:8080/x", &mut headers)
            .unwrap();
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_some(),
            "a loopback host matching the base must receive the token over http"
        );
    }

    // --- build() auth propagation ------------------------------------------------------------

    #[test]
    fn build_propagates_auth_token_and_scheme_to_request_config() {
        // CommonBuilderConfig::build() copies auth_token and auth_scheme from the builder into
        // the resolved RequestConfig so the shared apply_auth path can use them on both the
        // listing and download paths.
        let cfg = CommonBuilderConfig {
            current_version: Some("1.0.0".to_string()),
            bin_name: Some("mybin".to_string()),
            bin_path_in_archive: Some("mybin".to_string()),
            auth_token: Some("secrettoken".to_string()),
            auth_scheme: super::AuthScheme::Bearer,
            ..Default::default()
        };
        let built = cfg.build().expect("valid config must build");
        assert_eq!(
            built.request.auth_token.as_deref(),
            Some("secrettoken"),
            "build() must copy auth_token into request.auth_token"
        );
        assert_eq!(
            built.request.auth_scheme,
            super::AuthScheme::Bearer,
            "build() must copy auth_scheme into request.auth_scheme"
        );
    }

    // --- Symmetric async-only injected client ------------------------------------------------

    // Per-slot: injecting only an async client leaves the sync slot empty, so build_client still
    // builds the sync client from the custom roots (garbage cert -> cert_error set). The injected
    // async client is kept as-is.
    #[cfg(feature = "async")]
    #[test]
    fn build_client_injected_async_only_still_builds_sync_from_certs() {
        struct DummyAsyncClient;
        impl crate::http_client::AsyncHttpClient for DummyAsyncClient {
            fn get<'a>(
                &'a self,
                _url: &'a str,
                _headers: &'a crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> futures_util::future::BoxFuture<
                'a,
                crate::Result<Box<dyn crate::http_client::AsyncHttpResponse>>,
            > {
                unreachable!("not called in this test")
            }
        }
        let mut req = RequestConfig {
            async_client: Some(std::sync::Arc::new(DummyAsyncClient)),
            ..Default::default()
        };
        req.root_certificates
            .push(crate::tls::Certificate::from_pem(BAD_PEM_CERT.to_vec()));
        req.build_client();
        assert!(
            req.cert_error.is_some(),
            "the sync slot must attempt the cert-build even when an async client is injected"
        );
        assert!(
            req.async_client.is_some(),
            "the injected async client must be kept as-is"
        );
    }

    // --- End-to-end: CommonBuilderConfig::build with all slots injected + garbage cert ----------

    #[test]
    fn common_builder_config_build_with_injected_clients_skips_cert_error() {
        // CommonBuilderConfig::build() calls build_client() internally. When every compiled client
        // slot is injected, no slot needs building, so a garbage cert produces no cert_error and
        // build() succeeds.
        struct DummyClient;
        impl crate::http_client::HttpClient for DummyClient {
            fn get(
                &self,
                _url: &str,
                _headers: &crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> crate::Result<Box<dyn crate::http_client::HttpResponse>> {
                unreachable!("not called in this test")
            }
        }
        #[cfg(feature = "async")]
        struct DummyAsyncClient;
        #[cfg(feature = "async")]
        impl crate::http_client::AsyncHttpClient for DummyAsyncClient {
            fn get<'a>(
                &'a self,
                _url: &'a str,
                _headers: &'a crate::http_client::HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> futures_util::future::BoxFuture<
                'a,
                crate::Result<Box<dyn crate::http_client::AsyncHttpResponse>>,
            > {
                unreachable!("not called in this test")
            }
        }
        let mut builder = CommonBuilderConfig {
            current_version: Some("0.1.0".to_string()),
            bin_name: Some("app".to_string()),
            bin_path_in_archive: Some("app".to_string()),
            ..Default::default()
        };
        builder.request.client = Some(std::sync::Arc::new(DummyClient));
        #[cfg(feature = "async")]
        {
            builder.request.async_client = Some(std::sync::Arc::new(DummyAsyncClient));
        }
        builder
            .request
            .root_certificates
            .push(crate::tls::Certificate::from_pem(b"garbage".to_vec()));
        // build() must succeed because every compiled client slot is injected.
        let config = builder
            .build()
            .expect("injected clients must prevent cert_error from blocking build");
        assert!(
            config.request.cert_error.is_none(),
            "cert_error must be None when all client slots were injected"
        );
        assert!(
            config.request.client.is_some(),
            "the injected client must be present in the resolved config"
        );
    }
}

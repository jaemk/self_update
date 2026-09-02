/// Allows you to pull the version from your Cargo.toml at compile time as
/// `MAJOR.MINOR.PATCH_PKGVERSION_PRE`
#[macro_export]
macro_rules! cargo_crate_version {
    // -- Pulled from clap.rs src/macros.rs
    () => {
        env!("CARGO_PKG_VERSION")
    };
}

/// Emit the shared transport setters (`timeout`, `request_header`, `retries`) that write into
/// a `RequestConfig` reachable at `self.$path` — e.g. `request_config_setters!(common.request)`
/// for an `UpdateBuilder` or `request_config_setters!(request)` for a `ReleaseListBuilder`.
macro_rules! request_config_setters {
    ($($path:tt).+) => {
        /// Set a per-request timeout applied to every HTTP request this builder makes
        /// (release listing, and — for an `Update` — the download). Defaults to no timeout.
        pub fn timeout(&mut self, timeout: std::time::Duration) -> &mut Self {
            self.$($path).+.timeout = Some(timeout);
            self
        }

        /// Add an extra HTTP header sent on every request, e.g. for a proxy or gateway. May be
        /// called multiple times; a repeated header name overwrites the previous value.
        ///
        /// Accepts anything that converts into a header name/value, so both typed values and plain
        /// strings work: `.request_header("X-Foo", "bar")` or
        /// `.request_header(self_update::http::header::ACCEPT, "application/json")`. A name or value
        /// that is not a valid HTTP header is reported as an `Error::InvalidHeader` from
        /// `build()` rather than panicking here.
        pub fn request_header<N, V>(&mut self, name: N, value: V) -> &mut Self
        where
            N: ::core::convert::TryInto<crate::http_client::header::HeaderName>,
            V: ::core::convert::TryInto<crate::http_client::header::HeaderValue>,
        {
            self.$($path).+.insert_header(name, value);
            self
        }

        /// Number of times to retry a failed API request (release listing, single-release-by-tag
        /// fetches, and any other listing or lookup request) **and** the binary download's
        /// request-establishment phase, with exponential backoff (see
        /// [`retry_backoff`](Self::retry_backoff)). Defaults to `0` (no retries). Intended for
        /// transient failures, though any failed attempt (including a permanent one such as a 404)
        /// consumes the retry budget -- **with one exception**:
        /// [`Error::RateLimited`](crate::errors::Error::RateLimited) is never retried. The wait is
        /// the server's to dictate (`Retry-After`, or the reset header), and it can be far longer
        /// than any backoff this crate would apply, so the error is returned immediately and the
        /// decision to sleep, reschedule, or give up stays with the caller instead of being spent
        /// inside the loop.
        ///
        /// The download is retried only *before* any bytes are streamed to disk (a failure
        /// mid-stream is not retried, since it would corrupt the partially-written file).
        ///
        /// On the [`backends::custom`](crate::backends::custom) backend this affects only the
        /// crate-controlled **download**: the release *listing* is performed entirely by your
        /// [`ReleaseSource`](crate::ReleaseSource), so retries there are your source's
        /// responsibility.
        pub fn retries(&mut self, retries: u32) -> &mut Self {
            self.$($path).+.retries = retries;
            self
        }

        /// Configure the exponential retry backoff: `base` is the delay before the first retry and
        /// the delay doubles each subsequent attempt, clamped to never exceed `max`. Defaults to a
        /// `100ms` base and a `~3.2s` cap. Applies to listing/lookup requests and to the binary
        /// download's request-establishment phase (see [`retries`](Self::retries)); a mid-stream
        /// transfer failure is not retried. As with `retries`,
        /// [`Error::RateLimited`](crate::errors::Error::RateLimited) never consults this backoff: it
        /// is returned immediately, since the wait it carries (`Retry-After` or the reset header)
        /// is the server's to dictate and can be far longer than any backoff this crate would apply.
        pub fn retry_backoff(
            &mut self,
            base: std::time::Duration,
            max: std::time::Duration,
        ) -> &mut Self {
            self.$($path).+.retry_base_delay = base;
            self.$($path).+.retry_max_delay = max;
            self
        }

        /// Use a custom [`HttpClient`](crate::http_client::HttpClient) for every request (release
        /// listing and the download) instead of the client the crate builds per call. This is the
        /// canonical, client-agnostic injection seam: hand over any `Arc<dyn HttpClient>` (a test
        /// double, a wrapper around your application's client, etc.). The client-specific
        /// convenience setters (`reqwest_client` / `ureq_agent`) are thin wrappers over this.
        /// `.timeout()` and `.request_header()` still apply per request, but `HTTP(S)_PROXY` env and
        /// the crate's TLS feature are left to your client.
        pub fn http_client(
            &mut self,
            client: std::sync::Arc<dyn crate::http_client::HttpClient>,
        ) -> &mut Self {
            self.$($path).+.client = Some(client);
            self
        }

        /// Async sibling of [`http_client`](Self::http_client): a custom
        /// [`AsyncHttpClient`](crate::http_client::AsyncHttpClient) used by the `*_async` verbs.
        #[cfg(feature = "async")]
        pub fn http_client_async(
            &mut self,
            client: std::sync::Arc<dyn crate::http_client::AsyncHttpClient>,
        ) -> &mut Self {
            self.$($path).+.async_client = Some(client);
            self
        }

        /// Use a pre-built blocking [`reqwest::Client`](::reqwest::blocking::Client) for every
        /// request (release listing and the download) instead of the client the crate builds per
        /// call. Hand over a client when you need control the per-request knobs can't give —
        /// custom TLS roots / mTLS, connection pooling, redirect policy, proxy-with-auth — or to
        /// reuse your application's existing client. `.timeout()` and `.request_header()` still
        /// apply per request, but `HTTP(S)_PROXY` env and the crate's TLS feature are left to your
        /// client. Used by the blocking API; for the async path use `reqwest_async_client` (under
        /// the `async` feature). Thin wrapper over [`http_client`](Self::http_client).
        #[cfg(feature = "reqwest")]
        pub fn reqwest_client(&mut self, client: ::reqwest::blocking::Client) -> &mut Self {
            self.http_client(std::sync::Arc::new(
                crate::http_client::ReqwestClient::from(client),
            ))
        }

        /// Async sibling of [`reqwest_client`](Self::reqwest_client): a pre-built async
        /// [`reqwest::Client`](::reqwest::Client) used by the `*_async` verbs.
        #[cfg(feature = "async")]
        pub fn reqwest_async_client(&mut self, client: ::reqwest::Client) -> &mut Self {
            self.http_client_async(std::sync::Arc::new(
                crate::http_client::ReqwestAsyncClient::from(client),
            ))
        }

        /// Use a pre-built [`ureq::Agent`](::ureq::Agent) for every request instead of the agent
        /// the crate builds per call. The agent owns its own timeout / TLS / proxy config, so
        /// `.timeout()` does not apply to an injected agent (configure it on the agent); extra
        /// `.request_header()`s are still applied per request. Thin wrapper over
        /// [`http_client`](Self::http_client).
        #[cfg(feature = "ureq")]
        pub fn ureq_agent(&mut self, agent: ::ureq::Agent) -> &mut Self {
            self.http_client(std::sync::Arc::new(
                crate::http_client::UreqClient::from(agent),
            ))
        }

        /// Trust an additional TLS root CA certificate for every request (release listing and the
        /// download). Call multiple times to add more than one. Use this to reach a server behind a
        /// private/internal CA without injecting a whole pre-built client. A malformed certificate
        /// surfaces as an [`Error::InvalidCertificate`](crate::errors::Error::InvalidCertificate)
        /// from `build()`. Construct the argument with
        /// [`Certificate::from_pem`](crate::Certificate::from_pem) or
        /// [`Certificate::from_der`](crate::Certificate::from_der).
        ///
        /// The certificates apply per transport: a client injected via
        /// [`http_client`](Self::http_client) (or `http_client_async`) owns its own TLS and ignores
        /// these certificates, but the *other*, auto-built transport still trusts them.
        ///
        /// PEM certificate bytes are validated at `build()` on every backend. DER bytes are
        /// validated at `build()` on the reqwest backend, but on a ureq-only build a malformed DER
        /// certificate is surfaced at connection time instead.
        ///
        /// **ureq-only builds**: when the `reqwest` feature is disabled, the crate-built ureq client
        /// trusts *only* the supplied certificates (replacing whatever the default root set was).
        /// Supply all CA certificates you need, including any public roots. If what you actually
        /// want is the machine's own trust store -- the usual case behind an intercepting corporate
        /// proxy, whose CA is already installed there -- enable the `native-certs` feature and do
        /// not call this setter at all. For anything finer, inject a pre-built `ureq::Agent` via
        /// [`ureq_agent`](Self::ureq_agent) carrying its own merged root set.
        pub fn add_root_certificate(&mut self, cert: crate::Certificate) -> &mut Self {
            self.$($path).+.root_certificates.push(cert);
            self
        }

        /// Route every request (release listing and the download) through an HTTP proxy, given as
        /// a URL that may embed credentials: `http://user:pass@proxy.corp:8080`. This is the
        /// corporate-proxy-with-auth case that `HTTP_PROXY` / `HTTPS_PROXY` cannot cover when the
        /// proxy requires a password you would rather not put in the environment.
        ///
        /// Calling it more than once replaces the previous URL. An unparseable URL surfaces as an
        /// [`Error::InvalidProxy`](crate::errors::Error::InvalidProxy) from `build()`; the password
        /// is redacted from that error and from this builder's `Debug`, so neither leaks into logs.
        /// Only HTTP CONNECT proxies are supported (SOCKS is out of scope; use
        /// [`http_client`](Self::http_client) with a client of your own for that).
        ///
        /// Precedence and interaction with the environment:
        ///
        /// - A client injected via [`http_client`](Self::http_client) (or `http_client_async`,
        ///   `reqwest_client`, `ureq_agent`) owns its own proxy config, so this setter has no
        ///   effect on it; the *other*, auto-built transport still uses it.
        /// - On the reqwest client, `HTTP(S)_PROXY` / `NO_PROXY` stay active alongside this proxy
        ///   (reqwest tries its configured proxies in order, first match wins). To ignore the
        ///   environment entirely, inject a client built with `.no_proxy()`.
        /// - On a ureq-only build, the agent has a single proxy slot, so this proxy **replaces**
        ///   the env-var proxy rather than layering with it.
        pub fn proxy(&mut self, url: impl Into<String>) -> &mut Self {
            self.$($path).+.proxy = Some(url.into());
            self
        }

        /// Authorize an additional host to receive the auth token.
        ///
        /// By default the token set via `auth_token` is sent only to the backend's own API host, so
        /// a server-supplied asset `download_url` or pagination `Link` pointing at a different host
        /// does not receive the credential. If your release assets are served from a separate host
        /// (a CDN or artifact mirror) that legitimately needs the token, authorize it here. Call
        /// multiple times to add more than one. Matching is by host, case-insensitive; the request
        /// must still use `https` (loopback hosts may use http).
        pub fn allow_auth_host(&mut self, host: impl Into<String>) -> &mut Self {
            self.$($path).+.auth_hosts.push(host.into());
            self
        }

        /// Allow the auth token to be forwarded over plain `http` (not just `https`) to a
        /// host-matched request.
        ///
        /// The token is still only attached to the configured API host or an
        /// [`allow_auth_host`](Self::allow_auth_host) entry; this only lifts the `https` scheme
        /// requirement. It transmits the credential in cleartext, so use it only for a trusted
        /// internal network you control. Off by default.
        pub fn dangerously_allow_non_https_auth_forwarding(&mut self) -> &mut Self {
            self.$($path).+.allow_insecure_auth = true;
            self
        }
    };
}

/// Emit a full `impl `[`UpdateConfig`](crate::update::UpdateConfig) block holding the standard
/// field accessors that every backend shares.
///
/// Each backend's `Update` stores the same set of common fields, so the accessor bodies are
/// identical. This macro emits the whole `impl UpdateConfig for $t` block so the shared accessors
/// live in exactly one place; the backend-specific fetch methods go in a separate
/// `impl ReleaseUpdate for $t` block.
///
/// A backend that needs to override [`UpdateConfig::api_headers`] (github/gitlab/gitea) passes the
/// override as a trailing `{ … }` block, which is spliced into the same `impl` (a trait can only be
/// implemented once per type):
///
/// ```ignore
/// impl_update_config_accessors!(Update);                 // default api_headers
/// impl_update_config_accessors!(Update, {               // custom api_headers
///     fn api_headers(&self, auth_token: Option<&str>) -> Result<HeaderMap> { api_headers(auth_token) }
/// });
/// ```
macro_rules! impl_update_config_accessors {
    ($t:ty) => {
        impl_update_config_accessors!(@emit (impl crate::update::UpdateConfig for $t), {});
        impl_update_config_accessors!(@internals (impl crate::update::UpdateInternals for $t));
    };
    ($t:ty, { $($extra:tt)* }) => {
        impl_update_config_accessors!(@emit (impl crate::update::UpdateConfig for $t), { $($extra)* });
        impl_update_config_accessors!(@internals (impl crate::update::UpdateInternals for $t));
    };
    // Generic form for the custom `AsyncUpdate<S>`: a `where (...)` clause carries the bound.
    ($t:ty, where ( $($bound:tt)* )) => {
        impl_update_config_accessors!(
            @emit (impl<S> crate::update::UpdateConfig for $t where $($bound)*),
            {}
        );
        impl_update_config_accessors!(
            @internals (impl<S> crate::update::UpdateInternals for $t where $($bound)*)
        );
    };
    (@internals ($($header:tt)*)) => {
        $($header)* {
            fn request_timeout(&self) -> Option<std::time::Duration> {
                self.common.request.timeout
            }
            fn request_headers(&self) -> &crate::http_client::HeaderMap {
                &self.common.request.headers
            }
            fn request_config(&self) -> &crate::backends::common::RequestConfig {
                &self.common.request
            }
            fn request_client(&self) -> Option<std::sync::Arc<dyn crate::http_client::HttpClient>> {
                self.common.request.client.clone()
            }
            #[cfg(feature = "async")]
            fn request_async_client(
                &self,
            ) -> Option<std::sync::Arc<dyn crate::http_client::AsyncHttpClient>> {
                self.common.request.async_client.clone()
            }
            fn progress_callback(&self) -> Option<std::sync::Arc<crate::DynProgressFn>> {
                self.common.progress_callback.as_ref().map(|c| c.0.clone())
            }
            fn verify_callback(&self) -> Option<std::sync::Arc<crate::DynVerifyFn>> {
                self.common.verify.as_ref().map(|c| c.0.clone())
            }
            fn verify_archive_callback(&self) -> Option<std::sync::Arc<crate::DynVerifyFn>> {
                self.common.verify_archive.as_ref().map(|c| c.0.clone())
            }
            fn asset_matcher(&self) -> Option<std::sync::Arc<crate::DynAssetMatcher>> {
                self.common.asset_matcher.as_ref().map(|c| c.0.clone())
            }
            #[cfg(feature = "checksums")]
            fn verify_checksum(&self) -> Option<&crate::Checksum> {
                self.common.checksum.as_ref()
            }
            #[cfg(feature = "checksums")]
            fn checksum_from_asset(&self) -> Option<&str> {
                self.common.checksum_from_asset.as_deref()
            }
            #[cfg(feature = "checksums")]
            fn verify_release_digest(&self) -> bool {
                self.common.verify_release_digest
            }
            #[cfg(feature = "signatures")]
            fn verifying_keys(&self) -> &[crate::VerifyingKey] {
                &self.common.verifying_keys
            }
        }
    };
    (@emit ($($header:tt)*), { $($extra:tt)* }) => {
        $($header)* {
            $($extra)*

        fn current_version(&self) -> &str {
            &self.common.current_version
        }
        fn target(&self) -> &str {
            &self.common.target
        }
        fn release_tag(&self) -> Option<&str> {
            self.common.release_tag.as_deref()
        }
        fn asset_identifier(&self) -> Option<&str> {
            self.common.asset_identifier.as_deref()
        }
        fn bin_name(&self) -> &str {
            &self.common.bin_name
        }
        fn bin_install_path(&self) -> &std::path::Path {
            &self.common.bin_install_path
        }
        fn check_install_path_writable(&self) -> bool {
            self.common.check_install_path_writable
        }
        fn bin_path_in_archive(&self) -> &str {
            &self.common.bin_path_in_archive
        }
        fn bundle_path_in_archive(&self) -> Option<&str> {
            self.common.bundle_path_in_archive.as_deref()
        }
        fn bundle_install_path(&self) -> Option<&std::path::Path> {
            self.common.bundle_install_path.as_deref()
        }
        fn show_download_progress(&self) -> bool {
            self.common.show_download_progress
        }
        fn show_output(&self) -> bool {
            self.common.show_output
        }
        fn no_confirm(&self) -> bool {
            self.common.no_confirm
        }
        fn update_strategy(&self) -> crate::update::UpdateStrategy {
            self.common.update_strategy
        }
        fn show_release_notes(&self) -> bool {
            self.common.show_release_notes
        }
        #[cfg(feature = "progress-bar")]
        fn progress_template(&self) -> &str {
            &self.common.progress_template
        }
        #[cfg(feature = "progress-bar")]
        fn progress_chars(&self) -> &str {
            &self.common.progress_chars
        }
        fn auth_token(&self) -> Option<&str> {
            // Single source of truth: the resolved token lives on the request config, where
            // `apply_auth` reads it for both the listing and download paths.
            self.common.request.auth_token.as_deref()
        }
        }
    };
}

/// Emit the backend-independent `UpdateBuilder` setters shared by every backend.
///
/// Emit the inherent sync update verbs on a backend `Update`.
///
/// `build()` returns the concrete `Update` (not `Box<dyn ReleaseUpdate>`), so these inherent methods
/// let callers write `.build()?.update()?` without importing the sealed
/// [`ReleaseUpdate`](crate::ReleaseUpdate) trait. Each forwards to the trait impl.
macro_rules! impl_sync_update_verbs {
    ($t:ty) => {
        impl $t {
            /// Display release information and update the current binary to the latest release,
            /// pending confirmation. Returns a [`VersionStatus`](crate::VersionStatus). See
            /// [`ReleaseUpdate::update`](crate::ReleaseUpdate::update).
            pub fn update(&self) -> crate::Result<crate::VersionStatus> {
                <Self as crate::ReleaseUpdate>::update(self)
            }

            /// Same as [`update`](Self::update) but returns a [`ReleaseStatus`](crate::ReleaseStatus)
            /// with the full release details.
            pub fn update_extended(&self) -> crate::Result<crate::ReleaseStatus> {
                <Self as crate::ReleaseUpdate>::update_extended(self)
            }

            /// Fetch the single newest release (raw, unfiltered). See
            /// [`ReleaseUpdate::get_latest_release`](crate::ReleaseUpdate::get_latest_release).
            pub fn get_latest_release(&self) -> crate::Result<crate::Releases> {
                <Self as crate::ReleaseUpdate>::get_latest_release(self)
            }

            /// Fetch the releases newer than the current version. See
            /// [`ReleaseUpdate::get_newer_releases`](crate::ReleaseUpdate::get_newer_releases).
            pub fn get_newer_releases(&self) -> crate::Result<crate::Releases> {
                <Self as crate::ReleaseUpdate>::get_newer_releases(self)
            }

            /// Fetch details of the release matching `ver`. See
            /// [`ReleaseUpdate::get_release_version`](crate::ReleaseUpdate::get_release_version).
            pub fn get_release_version(&self, ver: &str) -> crate::Result<crate::Release> {
                <Self as crate::ReleaseUpdate>::get_release_version(self, ver)
            }

            /// Whether a release newer than the current version is available, returning it if so.
            ///
            /// A convenience over [`get_newer_releases`](Self::get_newer_releases): returns the
            /// newest strictly-newer [`Release`](crate::Release), or `None` when already up to date.
            ///
            /// Note that the returned release is the newest *available*, which is not necessarily
            /// the one [`update`](Self::update) would install: the update pipeline prefers the
            /// newest semver-*compatible* release and falls back to the newest available only when
            /// no compatible one exists.
            pub fn is_update_available(&self) -> crate::Result<Option<crate::Release>> {
                Ok(self.get_newer_releases()?.into_vec().into_iter().next())
            }
        }
    };
}

/// Emit the inherent async update verbs on a backend `AsyncUpdate(Update)` newtype.
///
/// `build_async()` returns the distinct `AsyncUpdate` newtype (not the blocking `Update`), so these
/// inherent `async` methods let callers write `.build_async()?.update_async().await` without
/// importing the sealed [`AsyncReleaseUpdate`](crate::AsyncReleaseUpdate) trait. Because the newtype
/// exposes *only* the async verbs, a stray blocking `.update()` on an async-built updater is a
/// compile error rather than a silent block of the executor. Each method forwards to the
/// [`AsyncReleaseUpdate`] impl on the inner blocking `Update` at `self.0`.
#[cfg(feature = "async")]
macro_rules! impl_async_update_verbs {
    ($t:ty) => {
        impl $t {
            /// Display release information and update the current binary to the latest release,
            /// pending confirmation. Returns a [`VersionStatus`](crate::VersionStatus). See
            /// [`AsyncReleaseUpdate::update_async`](crate::AsyncReleaseUpdate::update_async).
            pub async fn update_async(&self) -> crate::Result<crate::VersionStatus> {
                crate::AsyncReleaseUpdate::update_async(&self.0).await
            }

            /// Same as [`update_async`](Self::update_async) but returns a
            /// [`ReleaseStatus`](crate::ReleaseStatus) with the full release details.
            pub async fn update_extended_async(&self) -> crate::Result<crate::ReleaseStatus> {
                crate::AsyncReleaseUpdate::update_extended_async(&self.0).await
            }

            /// Fetch the single newest release (raw, unfiltered). See
            /// [`AsyncReleaseUpdate::get_latest_release_async`](crate::AsyncReleaseUpdate::get_latest_release_async).
            pub async fn get_latest_release_async(&self) -> crate::Result<crate::Releases> {
                crate::AsyncReleaseUpdate::get_latest_release_async(&self.0).await
            }

            /// Fetch the releases newer than the current version. See
            /// [`AsyncReleaseUpdate::get_newer_releases_async`](crate::AsyncReleaseUpdate::get_newer_releases_async).
            pub async fn get_newer_releases_async(&self) -> crate::Result<crate::Releases> {
                crate::AsyncReleaseUpdate::get_newer_releases_async(&self.0).await
            }

            /// Fetch details of the release matching `ver`. See
            /// [`AsyncReleaseUpdate::get_release_version_async`](crate::AsyncReleaseUpdate::get_release_version_async).
            pub async fn get_release_version_async(
                &self,
                ver: &str,
            ) -> crate::Result<crate::Release> {
                crate::AsyncReleaseUpdate::get_release_version_async(&self.0, ver).await
            }

            /// Whether a release newer than the current version is available, returning it if so.
            ///
            /// A convenience over [`get_newer_releases_async`](Self::get_newer_releases_async):
            /// returns the newest strictly-newer [`Release`](crate::Release), or `None` when already
            /// up to date.
            ///
            /// Note that the returned release is the newest *available*, which is not necessarily
            /// the one [`update_async`](Self::update_async) would install: the update pipeline
            /// prefers the newest semver-*compatible* release and falls back to the newest
            /// available only when no compatible one exists.
            pub async fn is_update_available_async(&self) -> crate::Result<Option<crate::Release>> {
                Ok(self
                    .get_newer_releases_async()
                    .await?
                    .into_vec()
                    .into_iter()
                    .next())
            }
        }
    };
}

/// Emit the environment-token surface of a builder: the `AUTH_TOKEN_ENV_VARS` list, the
/// `auth_token_from_env()` setter that resolves it, and the `has_auth_token()` query.
///
/// * `token:` — the path (relative to `self`) of the builder's token field: `common.auth_token` on
///   an `UpdateBuilder`, plain `auth_token` on a `ReleaseListBuilder`.
/// * `env_sourced:` — the path of the `bool` recording that the current token came from the
///   environment. Set here, cleared by every explicit `auth_token(..)` setter (via
///   [`set_explicit_auth_token`](crate::backends::common::set_explicit_auth_token)), and read by
///   `build()` — via [`env_token_host_decision`](crate::backends::common::env_token_host_decision)
///   — to warn (or, for a backend with no canonical host, withhold the token) when it would be bound
///   to an unacknowledged host.
/// * `vars:` — the env var names, in precedence order.
/// * `rationale:` — the backend-specific closing paragraph of the rustdoc, so github's
///   60/5000-requests-per-hour numbers are not rendered verbatim on backends where they are false.
///
/// Only the forge backends invoke this; the attribute keeps a build without any of them (e.g.
/// `--no-default-features --features "reqwest rustls s3"`) warning-free, matching the `cfg_attr`
/// gates on the `backends::common` helpers it calls.
#[cfg_attr(
    not(any(
        feature = "github",
        feature = "gitlab",
        feature = "gitea",
        feature = "gitee"
    )),
    allow(unused_macros)
)]
macro_rules! impl_auth_token_from_env {
    (
        token: $($field:ident).+,
        env_sourced: $($env_sourced:ident).+,
        vars: [$($var:literal),+ $(,)?],
        rationale: $rationale:literal $(,)?
    ) => {
        /// The environment variables [`auth_token_from_env`](Self::auth_token_from_env) reads, in
        /// precedence order. Crate-internal, and it is the very list the setter uses, so a test
        /// asserting it is asserting the real behavior.
        pub(crate) const AUTH_TOKEN_ENV_VARS: &'static [&'static str] = &[$($var),+];

        /// Set the authorization token from the environment, using the first of these variables
        /// that is set and non-empty (surrounding whitespace is trimmed), in this order:
        ///
        $(#[doc = concat!("- `", $var, "`")])+
        ///
        /// Reading the environment is **opt-in**: nothing here happens unless you call this. A
        /// library that harvests credentials on its own would be surprising, and the configured API
        /// base can be a self-hosted host, so the decision to send a token stays with your
        /// application.
        ///
        /// **Precedence:** an explicit [`auth_token`](Self::auth_token) always wins, in either call
        /// order — the environment is only a *fallback* for an unset token. So
        /// `auth_token(t).auth_token_from_env()` and `auth_token_from_env().auth_token(t)` both end
        /// up with `t`, and an ambient `*_TOKEN` in the environment can never displace the
        /// credential your application provisioned. When none of the variables is set, the token is
        /// left as it was and the request goes out exactly as before. Use
        /// [`has_auth_token`](Self::has_auth_token) to find out whether anything was picked up.
        ///
        /// "Safe to call unconditionally" has one caveat: a variable that is *set* but stale,
        /// expired, revoked, or scoped to a different resource makes the request **fail** (401/403)
        /// where an anonymous request would have succeeded against a public repository. That is a
        /// property of the environment, not of this call, but it means adding the call can turn a
        /// working anonymous fetch into a failing authenticated one.
        ///
        /// The lookup happens here, not at request time, so the resolved token does not depend on
        /// env changes made later in the process. Validity is *not* checked here: a value that
        /// cannot be encoded as an HTTP header surfaces as
        /// [`InvalidAuthToken`](crate::errors::Error::InvalidAuthToken) at **request** time,
        /// not from `build()`, and its message does not mention the environment — so check the
        /// variables above when you see it.
        ///
        /// The variable set does **not** change with a custom `api_base_url` / `host`: the same
        /// names are read whatever host you point the builder at. For a backend that has a
        /// canonical host, `build()` logs a warning when an env-sourced token would be bound to a
        /// different, unacknowledged one; a backend with no canonical host instead withholds the
        /// token in that case (see [`has_auth_token`](Self::has_auth_token)).
        ///
        #[doc = $rationale]
        pub fn auth_token_from_env(&mut self) -> &mut Self {
            // The closure form means the environment is only actually read (and its "using the auth
            // token from $X" / non-UTF-8 diagnostics only actually logged) when the slot is blank --
            // never for a call whose result would just be thrown away (A6).
            let filled = crate::backends::common::fill_env_token_if_unset_with(
                &mut self.$($field).+,
                || crate::backends::common::token_from_env(Self::AUTH_TOKEN_ENV_VARS),
            );
            if filled {
                self.$($env_sourced).+ = true;
            }
            self
        }

        /// Whether an authorization token is *configured* on this builder, from either
        /// [`auth_token`](Self::auth_token) or [`auth_token_from_env`](Self::auth_token_from_env).
        ///
        /// This is configuration, not a prediction: at request time the token is withheld unless the
        /// URL's host matches the configured API host or an `allow_auth_host` entry over https, and a
        /// user-supplied `Authorization` header via `request_header` takes precedence over it. On
        /// gitea an env-sourced token is additionally withheld unless the configured host was
        /// acknowledged.
        pub fn has_auth_token(&self) -> bool {
            !crate::backends::common::is_blank_token(self.$($field).+.as_deref())
        }
    };
}

/// Every backend's `UpdateBuilder` embeds a `common:
/// crate::backends::common::CommonBuilderConfig` field; these setters write through it, so
/// the shared configuration surface (target, identifier, bin name/path, version, progress
/// style, auth token, verifying keys) lives in exactly one place. The macro is invoked
/// inside each `impl UpdateBuilder` block; backend-specific setters (repo/host/url, bucket,
/// region, credentials) are written per backend.
macro_rules! impl_common_builder_setters {
    // Every shared setter, plus `auth_token`, the backend's conventional env-var lookup
    // (`auth_token_from_env`), its `has_auth_token` query and its `AUTH_TOKEN_ENV_VARS` list.
    // `$var`s are the env var names in precedence order; `$rationale` is the backend-specific
    // closing rustdoc paragraph.
    //
    // This is the ONLY arm that emits `auth_token`. A bare, env-lookup-less
    // `impl_common_builder_setters!()` arm used to exist for that purpose, but every forge backend
    // (github/gitlab/gitea/gitee) has an env-var convention and uses THIS arm, and every non-forge
    // backend (custom x2, s3, manifest) uses `no_auth_token` instead -- so the bare arm was dead
    // code (it emitted an `auth_token` setter with an empty `#[doc = ""]` and no cross-link) and has
    // been removed, along with the `@auth_token $extra` indirection it needed. One consequence worth
    // being deliberate about: `has_auth_token()` is therefore emitted ONLY by
    // `impl_auth_token_from_env!` below -- a builder cannot offer "is a token set?" without also
    // opting into the environment lookup.
    (auth_env: [$($var:literal),+ $(,)?], rationale: $rationale:literal $(,)?) => {
        impl_common_builder_setters!(@shared);

        /// Set the authorization token, used in requests to the backend's api url.
        ///
        /// This is to support private repos where you need an auth token.
        /// **Make sure not to bake the token into your app**; it is recommended you obtain
        /// it via another mechanism, such as environment variables or prompting the user.
        ///
        /// The value is stored verbatim -- it is **not** trimmed, unlike
        /// [`auth_token_from_env`](Self::auth_token_from_env)'s environment lookup (which trims
        /// surrounding whitespace). So `auth_token(" ghp_x\n")` keeps the leading space and
        /// trailing newline and fails at **request** time as
        /// [`Error::InvalidAuthToken`](crate::errors::Error::InvalidAuthToken), where the same raw
        /// value in an environment variable would have worked. A **blank** value (empty, or all
        /// whitespace) is different: it is treated as unset by
        /// [`has_auth_token`](Self::has_auth_token), is never sent as a request header, and does not
        /// block [`auth_token_from_env`](Self::auth_token_from_env)'s fallback -- so
        /// `auth_token("").auth_token_from_env()` still picks up the environment.
        ///
        /// A blank value is the one case where the two setters are **order sensitive**: this setter
        /// always overwrites the slot, so `auth_token_from_env().auth_token("")` discards the
        /// resolved token and the request goes out unauthenticated (`has_auth_token()` then reports
        /// `false`). A real, non-blank token wins in either order, as below.
        ///
        /// The token can also be taken from the environment with
        /// [`auth_token_from_env`](Self::auth_token_from_env), which reads the backend's
        /// conventional variables. This setter always wins over that one, in either call order.
        pub fn auth_token(&mut self, auth_token: impl Into<String>) -> &mut Self {
            crate::backends::common::set_explicit_auth_token(
                &mut self.common.auth_token,
                &mut self.common.auth_token_from_env,
                auth_token,
            );
            self
        }

        impl_auth_token_from_env!(
            token: common.auth_token,
            env_sourced: common.auth_token_from_env,
            vars: [$($var),+],
            rationale: $rationale,
        );
    };
    // Variant for backends that don't authenticate via a bearer token (e.g. s3, which uses
    // `access_key`/SigV4). Omits the shared `auth_token` setter so the backend can either drop
    // it or provide its own (e.g. a `#[deprecated]` no-op pointing at the real knob).
    (no_auth_token) => {
        impl_common_builder_setters!(@shared);
    };
    (@shared) => {
        /// Required. Set the current app version, used to compare against the latest available
        /// version. The `cargo_crate_version!` macro can be used to pull the version from your
        /// `Cargo.toml`
        pub fn current_version(&mut self, ver: impl Into<String>) -> &mut Self {
            self.common.current_version = Some(ver.into());
            self
        }

        /// Set the release tag to update to.
        ///
        /// Pass the tag exactly as it appears in the remote (including any leading `v`, e.g.
        /// `"v1.2.3"`) — it is used verbatim to look the release up by tag. If not specified, the
        /// latest available release is used. (Note that the `{{ version }}` substitution in
        /// [`bin_path_in_archive`](Self::bin_path_in_archive) is still the bare semver with any
        /// leading `v` stripped, regardless of what is passed here.)
        ///
        /// The tag must resolve to a semver version after stripping a leading `v`: pinning a
        /// rolling tag like `nightly` or a date tag fails at update time with an
        /// [`Error::SemVer`](crate::errors::Error::SemVer) naming the tag. (In release
        /// *listings* such tags are skipped instead, so a repo mixing rolling and versioned
        /// releases stays updatable.)
        pub fn release_tag(&mut self, ver: impl Into<String>) -> &mut Self {
            self.common.release_tag = Some(ver.into());
            self
        }

        /// Set the target triple that will be downloaded, e.g. `x86_64-unknown-linux-gnu`.
        ///
        /// If unspecified, the build target of the crate will be used.
        pub fn target(&mut self, target: impl Into<String>) -> &mut Self {
            self.common.target = Some(target.into());
            self
        }

        /// Set the identifiable token for the asset in case of multiple compatible assets.
        ///
        /// If unspecified, the first asset matching the target will be chosen.
        pub fn asset_identifier(&mut self, identifier: impl Into<String>) -> &mut Self {
            self.common.asset_identifier = Some(identifier.into());
            self
        }

        /// Required. Set the exe's name. Also derives `bin_path_in_archive` (with the platform
        /// executable suffix appended) unless you called
        /// [`bin_path_in_archive`](Self::bin_path_in_archive) explicitly.
        ///
        /// Re-calling `bin_name` re-derives `bin_path_in_archive` (each call wins over the
        /// previous auto-derive). An explicit [`bin_path_in_archive`](Self::bin_path_in_archive)
        /// call blocks the auto-derive: calling `bin_name` after it will **not** overwrite your
        /// explicit path.
        ///
        /// This method appends the platform-specific executable suffix
        /// (`std::env::consts::EXE_SUFFIX`) to the name when it is absent.
        pub fn bin_name(&mut self, name: impl Into<String>) -> &mut Self {
            let name = name.into();
            let raw_bin_name = format!(
                "{}{}",
                name.trim_end_matches(std::env::consts::EXE_SUFFIX),
                std::env::consts::EXE_SUFFIX
            );
            // Overwrite the archive path only when it is unset or was previously auto-derived (not
            // explicitly set by the caller). An explicit `bin_path_in_archive(...)` call sets
            // `bin_path_in_archive_auto = false`, making that value sticky even across re-calls to
            // `bin_name`.
            if self.common.bin_path_in_archive.is_none() || self.common.bin_path_in_archive_auto {
                self.common.bin_path_in_archive = Some(raw_bin_name.clone());
                self.common.bin_path_in_archive_auto = true;
            }
            self.common.bin_name = Some(raw_bin_name);
            self
        }

        /// Set the installation path for the new exe, defaults to the current
        /// executable's path.
        pub fn bin_install_path<A: AsRef<std::path::Path>>(
            &mut self,
            bin_install_path: A,
        ) -> &mut Self {
            self.common.bin_install_path =
                Some(std::path::PathBuf::from(bin_install_path.as_ref()));
            self
        }

        /// Opt-in preflight: probe whether `bin_install_path` is writable *before* anything is
        /// downloaded, failing fast with
        /// [`Error::InstallPathNotWritable`](crate::errors::Error::InstallPathNotWritable) when it
        /// is definitely not (so a long download is not wasted only to hit a permission error at
        /// the final replace step). Defaults to `false` (off).
        ///
        /// The probe is conservative: only a definite permission refusal errors. Indeterminate
        /// results (a missing parent directory, an unusual filesystem, any non-permission IO error)
        /// are treated as "proceed" and let the real install step surface the outcome. It never
        /// escalates privileges. Regardless of this setting, the install step always annotates a
        /// permission failure as `InstallPathNotWritable` naming the path.
        pub fn check_install_path_writable(&mut self, check: bool) -> &mut Self {
            self.common.check_install_path_writable = check;
            self
        }

        /// Set the path of the exe inside the release tarball. This is the location of the
        /// executable relative to the base of the tar'd directory and is the path that will
        /// be copied to the `bin_install_path`. If not specified, this will default to the
        /// value of `bin_name`. This only needs to be specified if the path to the binary
        /// (from the root of the tarball) is not equal to just the `bin_name`.
        ///
        /// This also supports variable paths:
        /// - `{{ bin }}` is replaced with the value of `bin_name`
        /// - `{{ target }}` is replaced with the value of `target`
        /// - `{{ version }}` is replaced with the resolved release version — the bare semver of the
        ///   release that the update actually installs, with any leading `v` stripped (e.g. `1.2.3`
        ///   for a `v1.2.3` tag) — regardless of the raw `release_tag` you configured.
        ///
        /// For example, a value of `"{{ target }}-{{ version }}-bin/{{ bin }}"` extracts the
        /// `bin` from a `target`/`version`-named subdirectory of the archive.
        ///
        /// Once called, subsequent [`bin_name`](Self::bin_name) calls will **not** overwrite this
        /// value (the explicit path is sticky). Call this method after `bin_name` to override the
        /// auto-derived path.
        pub fn bin_path_in_archive(&mut self, bin_path: impl Into<String>) -> &mut Self {
            self.common.bin_path_in_archive = Some(bin_path.into());
            // An explicit set wins and is sticky: a subsequent `bin_name` call must not re-derive.
            self.common.bin_path_in_archive_auto = false;
            self
        }

        /// Install a whole directory bundle (a macOS `.app`) instead of a single executable,
        /// naming the bundle directory *inside the archive*, relative to its root (e.g.
        /// `"MyApp.app"` or `"{{ bin }}-{{ version }}/MyApp.app"`).
        ///
        /// Calling this selects bundle mode: the archive is extracted in full and the named
        /// directory replaces [`bundle_install_path`](Self::bundle_install_path) as one unit, so a
        /// bundle's resources and code signature stay consistent with its executable (replacing
        /// only the exe inside an `.app` leaves stale resources and breaks the signature).
        ///
        /// The same `{{ bin }}` / `{{ target }}` / `{{ version }}` substitutions as
        /// [`bin_path_in_archive`](Self::bin_path_in_archive) apply.
        ///
        /// Bundle mode replaces a directory rather than a file, so combining it with an explicit
        /// [`bin_install_path`](Self::bin_install_path) or
        /// [`bin_path_in_archive`](Self::bin_path_in_archive) is rejected by `build()` with
        /// [`Error::ConflictingConfig`](crate::errors::Error::ConflictingConfig) rather than
        /// silently ignoring one of them. [`bin_name`](Self::bin_name) is still required (it names
        /// the asset and feeds `{{ bin }}`); the path it auto-derives is simply unused.
        ///
        /// The replacement is a whole-tree swap: the new bundle is staged next to the destination,
        /// the old tree is stashed, and the two are renamed, so a failure at any step restores the
        /// original bundle. When the running executable lives inside the bundle it is renamed aside
        /// first (the mechanism a self-replace relies on), so no running image remains in the old
        /// tree; after a successful update the running exe's path holds the new executable and the
        /// process can relaunch itself with [`restart`](crate::restart::restart).
        ///
        /// Phase A targets macOS `.app` bundles. A directory bundle on linux or windows is swapped
        /// by the same code path, but on windows the swap fails (and rolls back) if the process
        /// holds files inside the bundle open beyond its own executable, for example a DLL loaded
        /// from it.
        ///
        /// See the crate-level "Bundle installs" section for a full example.
        pub fn bundle_path_in_archive(&mut self, bundle_path: impl Into<String>) -> &mut Self {
            self.common.bundle_path_in_archive = Some(bundle_path.into());
            self
        }

        /// Set the installed bundle directory that bundle mode replaces, e.g.
        /// `"/Applications/MyApp.app"`. Requires
        /// [`bundle_path_in_archive`](Self::bundle_path_in_archive), which is what selects bundle
        /// mode; setting this alone is an
        /// [`Error::MissingField`](crate::errors::Error::MissingField) from `build()` rather than a
        /// silently discarded install path.
        ///
        /// A symlinked path is resolved to the tree it points at, so the installed bundle behind the
        /// link is what gets replaced and the link itself survives the update.
        ///
        /// On macOS this defaults to the nearest `.app` ancestor of the running executable, so an
        /// app launched from `/Applications/MyApp.app/Contents/MacOS/myapp` updates itself in
        /// place; a running executable with no `.app` ancestor is an
        /// [`Error::NoAppBundle`](crate::errors::Error::NoAppBundle) from `build()`, and a
        /// quarantined app running from a read-only translocated mount is an
        /// [`Error::AppTranslocated`](crate::errors::Error::AppTranslocated). On every other
        /// platform there is no default and bundle mode requires this setter.
        ///
        /// The swap stages the new tree inside this path's parent directory, so the parent must be
        /// writable and hold enough free space for one more copy of the bundle. There is no
        /// cross-filesystem fallback (staging in the parent makes one unnecessary), and no
        /// privilege escalation: an unwritable `/Applications` surfaces as
        /// [`Error::InstallPathNotWritable`](crate::errors::Error::InstallPathNotWritable).
        pub fn bundle_install_path<A: AsRef<std::path::Path>>(
            &mut self,
            bundle_install_path: A,
        ) -> &mut Self {
            self.common.bundle_install_path =
                Some(std::path::PathBuf::from(bundle_install_path.as_ref()));
            self
        }

        /// Toggle download progress bar, defaults to `off`.
        pub fn show_download_progress(&mut self, show: bool) -> &mut Self {
            self.common.show_download_progress = show;
            self
        }

        /// Set download progress style, as a typed [`ProgressStyle`](crate::ProgressStyle)
        /// (template + chars) so the two strings can't be transposed.
        #[cfg(feature = "progress-bar")]
        pub fn progress_style(&mut self, style: crate::ProgressStyle) -> &mut Self {
            self.common.progress_template = style.template;
            self.common.progress_chars = style.chars;
            self
        }

        /// Toggle update output information, defaults to `true`.
        ///
        /// Unattended/daemon/CI callers usually want `.show_output(false)`. Note the
        /// release-status block is still printed when an interactive confirmation is pending (the
        /// default), since it is shown *before* the confirmation prompt, so fully silencing output
        /// also requires `.no_confirm(true)` (see [`no_confirm`](Self::no_confirm)).
        pub fn show_output(&mut self, show: bool) -> &mut Self {
            self.common.show_output = show;
            self
        }

        /// Toggle download confirmation. Defaults to `false` (interactive: the update prompts
        /// "Do you want to continue?" and blocks on stdin).
        ///
        /// **Unattended/daemon/CI callers must set `.no_confirm(true)`** or the update will block
        /// forever waiting for input; they usually also set `.show_output(false)`. Note the
        /// release-status block is printed *before* this confirmation prompt, so silencing it
        /// requires `show_output(false)` as well.
        pub fn no_confirm(&mut self, no_confirm: bool) -> &mut Self {
            self.common.no_confirm = no_confirm;
            self
        }

        /// Choose which release the unpinned "latest" path installs when several are newer than the
        /// current version. Defaults to [`UpdateStrategy::Compatible`](crate::UpdateStrategy::Compatible)
        /// (prefer the newest semver-compatible release); pass
        /// [`UpdateStrategy::Latest`](crate::UpdateStrategy::Latest) to always jump to the newest
        /// release, even across an incompatible (major) bump. No effect when a `release_tag(..)` is
        /// pinned.
        pub fn update_strategy(&mut self, strategy: crate::update::UpdateStrategy) -> &mut Self {
            self.common.update_strategy = strategy;
            self
        }

        /// Show the release notes in the confirmation prompt (defaults to `false`). When enabled,
        /// the release status block includes the release notes URL if the backend provides one
        /// (github/gitlab/gitea fill it from the release page; see
        /// [`Release::release_notes_url`](crate::Release::release_notes_url)), otherwise the release
        /// body if present. No effect when `no_confirm` and `show_output` are both off (nothing is
        /// printed).
        pub fn show_release_notes(&mut self, show: bool) -> &mut Self {
            self.common.show_release_notes = show;
            self
        }

        /// Configure for unattended/CI use: disables interactive confirmation (`no_confirm(true)`)
        /// and suppresses status output (`show_output(false)`) in one call. Without this, the
        /// default (`no_confirm == false`) blocks on stdin waiting for a "y" confirmation.
        pub fn unattended(&mut self) -> &mut Self {
            self.common.no_confirm = true;
            self.common.show_output = false;
            self
        }

        request_config_setters!(common.request);

        /// Register a callback invoked as the release downloads, with
        /// `(bytes_downloaded_so_far, total_bytes)` (`total_bytes` is `None` when the server
        /// sends no `Content-Length`). Independent of `show_download_progress`; use it to drive
        /// a GUI or structured logging. The callback is `Fn`, so track state via interior
        /// mutability (e.g. an `AtomicU64` or a channel).
        pub fn progress_callback(
            &mut self,
            callback: impl Fn(u64, Option<u64>) + Send + Sync + 'static,
        ) -> &mut Self {
            self.common.progress_callback =
                Some(crate::ProgressCallback(std::sync::Arc::new(callback)));
            self
        }

        /// Override how the release asset to download is selected. The closure receives the
        /// release's assets and returns the one to download (or `None` to fail the update with
        /// "no asset found"). When set, this **replaces** the built-in `target`/`identifier`
        /// substring matching — useful for releases whose asset names the default heuristic
        /// can't express. The closure is `Fn` and may be called once per update.
        pub fn asset_matcher(
            &mut self,
            matcher: impl Fn(&[crate::ReleaseAsset]) -> Option<crate::ReleaseAsset>
                + Send
                + Sync
                + 'static,
        ) -> &mut Self {
            self.common.asset_matcher = Some(crate::AssetMatcher(std::sync::Arc::new(matcher)));
            self
        }

        /// Register a post-update verification hook. After the new binary is extracted but
        /// **before** it replaces the installed one, the closure is called with the path to the
        /// extracted binary; returning `Err(..)` aborts the update (nothing is installed), so a bad
        /// release cannot replace a working binary. Typical use: run `new --version` and check it,
        /// returning `Ok(())` on success or an error describing the rejection.
        ///
        /// This runs **last** in the verification chain and on the **extracted binary**, not the
        /// downloaded archive. The full order is: [`verify_checksum`](Self::verify_checksum) (digest
        /// of the archive) -> release digest ([`verify_release_digest`](Self::verify_release_digest),
        /// over the archive) -> signature ([`verifying_keys`](Self::verifying_keys), over the archive) ->
        /// [`verify_archive`](Self::verify_archive) (the downloaded archive) -> extract ->
        /// `verify_binary` (the extracted binary) -> replace. Use
        /// `verify_checksum`/`verifying_keys`/`verify_archive` to gate the download by content; use
        /// `verify_binary` to gate it by running the new binary. Reject with
        /// [`Error::verification_rejected("reason")`](crate::Error::verification_rejected), which is
        /// surfaced as-is; any other returned error's message becomes the reason of the resulting
        /// `Error::VerificationRejected`.
        pub fn verify_binary(
            &mut self,
            verify: impl Fn(&std::path::Path) -> crate::Result<()> + Send + Sync + 'static,
        ) -> &mut Self {
            self.common.verify = Some(crate::VerifyCallback(std::sync::Arc::new(verify)));
            self
        }

        /// Register a pre-extraction verification hook over the **downloaded archive**. After the
        /// archive is downloaded and every content gate the crate itself applies has passed, but
        /// **before** anything is extracted, the closure is called with the path to the downloaded
        /// file; returning `Err(..)` aborts the update with nothing extracted and nothing installed.
        ///
        /// This is the hook for verification that must run against the artifact **as published** --
        /// an external attestation or signature check whose subject is the release file itself, such
        /// as `gh attestation verify <archive> --repo owner/repo`, `cosign verify-blob`, or a
        /// detached-signature check the crate has no built-in support for. Verifying the extracted
        /// binary instead ([`verify_binary`](Self::verify_binary)) would ask about a different file
        /// than the one the forge attested.
        ///
        /// Where it sits in the chain: [`verify_checksum`](Self::verify_checksum) (digest of the
        /// archive) -> release digest ([`verify_release_digest`](Self::verify_release_digest), over
        /// the archive) -> signature ([`verifying_keys`](Self::verifying_keys), over the archive) ->
        /// `verify_archive` (the downloaded archive) -> extract ->
        /// [`verify_binary`](Self::verify_binary) (the extracted binary) -> replace. Running last
        /// among the archive gates means the cheap built-in digest checks reject a corrupt download
        /// before an external tool is spawned on it. Bundle mode
        /// ([`bundle_path_in_archive`](Self::bundle_path_in_archive)) runs it at the same point.
        ///
        /// Reject with
        /// [`Error::archive_verification_rejected("reason")`](crate::Error::archive_verification_rejected),
        /// which is surfaced as-is; any other returned error's message becomes the reason of the
        /// resulting [`Error::ArchiveVerificationRejected`](crate::Error::ArchiveVerificationRejected).
        /// That is a distinct variant from the `verify_binary` hook's
        /// [`Error::VerificationRejected`](crate::Error::VerificationRejected), so a caller using
        /// both hooks can tell which one rejected the update.
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
        pub fn verify_archive(
            &mut self,
            verify: impl Fn(&std::path::Path) -> crate::Result<()> + Send + Sync + 'static,
        ) -> &mut Self {
            self.common.verify_archive = Some(crate::VerifyCallback(std::sync::Arc::new(verify)));
            self
        }

        /// Verify the downloaded artifact against an expected [`Checksum`](crate::Checksum)
        /// (e.g. one published in a `SHA256SUMS` file) before installing it. The algorithm is
        /// chosen by the `Checksum` variant.
        ///
        /// Independent of [`verify_release_digest`](Self::verify_release_digest): when both apply,
        /// both must pass.
        #[cfg(feature = "checksums")]
        pub fn verify_checksum(&mut self, checksum: crate::Checksum) -> &mut Self {
            self.common.checksum = Some(checksum);
            self
        }

        /// Verify the downloaded artifact against a digest published in a sums asset of the same
        /// release, naming that asset (e.g. `"SHA256SUMS"`).
        ///
        /// During the update, after the artifact to install has been selected and confirmed and
        /// before it is downloaded, the named asset is fetched over the same transport (client,
        /// headers, auth, timeout, retries) and the entry matching the selected asset's file name
        /// supplies the expected digest. The algorithm comes from the digest's length, so a
        /// `SHA512SUMS` asset needs no extra configuration, and the usual formats are accepted (see
        /// [`Checksum::from_sums_file`](crate::Checksum::from_sums_file) for the exact set). It
        /// costs one extra request, taken before the artifact download so a missing or unusable
        /// sums asset fails without pulling the whole release first.
        ///
        /// The failure modes before verification (no such asset in the release, no entry for the
        /// selected asset, an unusable digest) are
        /// [`Error::ChecksumSourceInvalid`](crate::Error::ChecksumSourceInvalid), distinct from the
        /// `ChecksumMismatch` a resolved-but-wrong digest produces; a release that publishes no
        /// sums asset is therefore an error rather than a silently skipped check.
        ///
        /// Independent of [`verify_checksum`](Self::verify_checksum) and
        /// [`verify_release_digest`](Self::verify_release_digest): when more than one applies, all
        /// of them must pass.
        ///
        /// ```rust
        /// # #[cfg(feature = "checksums")]
        /// # fn f() -> Result<(), Box<dyn std::error::Error>> {
        /// self_update::backends::github::Update::configure()
        ///     .repo_owner("jaemk")
        ///     .repo_name("self_update")
        ///     .bin_name("github")
        ///     .current_version(self_update::cargo_crate_version!())
        ///     .checksum_from_asset("SHA256SUMS")
        ///     .build()?
        ///     .update()?;
        /// # Ok(())
        /// # }
        /// ```
        #[cfg(feature = "checksums")]
        pub fn checksum_from_asset(&mut self, asset_name: impl Into<String>) -> &mut Self {
            self.common.checksum_from_asset = Some(asset_name.into());
            self
        }

        /// Verify the downloaded artifact against the digest the backend publishes for the
        /// selected asset (github's per-asset `digest` field, `sha256:<hex>`), before installing
        /// it. **On by default** whenever the `checksums` feature is enabled; pass `false` to opt
        /// out.
        ///
        /// The check only runs when the selected asset actually carries a digest — github fills
        /// it, the other backends don't (their APIs publish none), so it is a no-op there. A
        /// digest that is present but malformed or uses an unsupported algorithm fails the update
        /// (loudly, rather than silently skipping); opting out is the escape hatch if a forge
        /// starts publishing digests this crate can't parse.
        ///
        /// Note this is an *integrity* check, not authenticity: the forge recomputes the digest
        /// if an asset is replaced. Use the `signatures` feature
        /// ([`verifying_keys`](Self::verifying_keys)) to verify authorship. Independent of
        /// [`verify_checksum`](Self::verify_checksum): when both apply, both must pass.
        #[cfg(feature = "checksums")]
        pub fn verify_release_digest(&mut self, verify: bool) -> &mut Self {
            self.common.verify_release_digest = verify;
            self
        }

        /// Specify the set of ed25519ph verifying keys used to validate a download's authenticity.
        ///
        /// Signature verification runs only when the `signatures` feature is enabled **and** at
        /// least one key is provided; a download then has to match one of the keys. Passing an
        /// empty set (or never calling this) leaves signature verification **disabled** — it is not
        /// an error, so don't rely on this as your only integrity check unless you know a key is
        /// always supplied.
        ///
        /// This **replaces** the key set on each call (unlike [`request_header`](Self::request_header),
        /// which appends); the last call wins.
        #[cfg(feature = "signatures")]
        pub fn verifying_keys(
            &mut self,
            keys: impl Into<Vec<crate::VerifyingKey>>,
        ) -> &mut Self {
            self.common.verifying_keys = keys.into();
            self
        }
    };
}

/// Helper to `print!` and immediately `flush` `stdout`
macro_rules! print_flush {
    ($literal:expr) => {
        print!($literal);
        ::std::io::Write::flush(&mut ::std::io::stdout())?;
    };
    ($literal:expr, $($arg:expr),*) => {
        print!($literal, $($arg),*);
        ::std::io::Write::flush(&mut ::std::io::stdout())?;
    }
}

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
        /// consumes the retry budget.
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
        /// transfer failure is not retried.
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
        /// trusts *only* the supplied certificates (replacing the default Mozilla root set). Supply
        /// all CA certificates you need, including any public roots. If you need the Mozilla set plus
        /// a custom CA, inject a pre-built `ureq::Agent` via [`ureq_agent`](Self::ureq_agent)
        /// configured with `RootCerts::PlatformVerifier` or a merged root set instead.
        pub fn add_root_certificate(&mut self, cert: crate::Certificate) -> &mut Self {
            self.$($path).+.root_certificates.push(cert);
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
            fn asset_matcher(&self) -> Option<std::sync::Arc<crate::DynAssetMatcher>> {
                self.common.asset_matcher.as_ref().map(|c| c.0.clone())
            }
            #[cfg(feature = "checksums")]
            fn verify_checksum(&self) -> Option<&crate::Checksum> {
                self.common.checksum.as_ref()
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
        fn bin_path_in_archive(&self) -> &str {
            &self.common.bin_path_in_archive
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

/// Every backend's `UpdateBuilder` embeds a `common:
/// crate::backends::common::CommonBuilderConfig` field; these setters write through it, so
/// the shared configuration surface (target, identifier, bin name/path, version, progress
/// style, auth token, verifying keys) lives in exactly one place. The macro is invoked
/// inside each `impl UpdateBuilder` block; backend-specific setters (repo/host/url, bucket,
/// region, credentials) are written per backend.
macro_rules! impl_common_builder_setters {
    // Default: every shared setter, including `auth_token`.
    () => {
        impl_common_builder_setters!(@shared);

        /// Set the authorization token, used in requests to the backend's api url.
        ///
        /// This is to support private repos where you need an auth token.
        /// **Make sure not to bake the token into your app**; it is recommended you obtain
        /// it via another mechanism, such as environment variables or prompting the user.
        pub fn auth_token(&mut self, auth_token: impl Into<String>) -> &mut Self {
            self.common.auth_token = Some(auth_token.into());
            self
        }
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
        /// extract -> `verify_binary` (the extracted binary) -> replace. Use
        /// `verify_checksum`/`verifying_keys` to gate the download by content; use `verify_binary` to
        /// gate it by running the new binary. Reject with
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

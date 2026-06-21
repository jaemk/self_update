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
        /// that is not a valid HTTP header is reported as an `Error::Config` from
        /// [`build()`](Self::build) rather than panicking here.
        pub fn request_header<N, V>(&mut self, name: N, value: V) -> &mut Self
        where
            N: ::core::convert::TryInto<crate::http_client::header::HeaderName>,
            V: ::core::convert::TryInto<crate::http_client::header::HeaderValue>,
        {
            self.$($path).+.insert_header(name, value);
            self
        }

        /// Number of times to retry a failed API request (release listing, single-release-by-tag
        /// fetches, and any other listing or lookup request), with exponential backoff. Defaults to
        /// `0` (no retries). Intended for transient failures, though any failed attempt (including
        /// a permanent one such as a 404) consumes the retry budget. The binary **download** is not
        /// retried — this knob does not affect it.
        ///
        /// **No-op on the custom backend.** It only ever retried the crate's built-in
        /// release-listing requests; on
        /// [`backends::custom`](crate::backends::custom) the listing is performed entirely by your
        /// [`ReleaseSource`](crate::ReleaseSource), so this setter has no effect there. Configure
        /// retries inside your source implementation instead.
        pub fn retries(&mut self, retries: u32) -> &mut Self {
            self.$($path).+.retries = retries;
            self
        }

        /// Use a pre-built blocking [`reqwest::Client`](::reqwest::blocking::Client) for every
        /// request (release listing and the download) instead of the client the crate builds per
        /// call. Hand over a client when you need control the per-request knobs can't give —
        /// custom TLS roots / mTLS, connection pooling, redirect policy, proxy-with-auth — or to
        /// reuse your application's existing client. `.timeout()` and `.request_header()` still
        /// apply per request, but `HTTP(S)_PROXY` env and the crate's TLS feature are left to your
        /// client. Used by the blocking API; for the async path use `reqwest_async_client` (under
        /// the `async` feature).
        #[cfg(feature = "reqwest")]
        pub fn reqwest_client(&mut self, client: ::reqwest::blocking::Client) -> &mut Self {
            self.$($path).+.client.blocking = Some(client);
            self
        }

        /// Async sibling of [`reqwest_client`](Self::reqwest_client): a pre-built async
        /// [`reqwest::Client`](::reqwest::Client) used by the `*_async` verbs.
        #[cfg(feature = "async")]
        pub fn reqwest_async_client(&mut self, client: ::reqwest::Client) -> &mut Self {
            self.$($path).+.client.r#async = Some(client);
            self
        }

        /// Use a pre-built [`ureq::Agent`](::ureq::Agent) for every request instead of the agent
        /// the crate builds per call. The agent owns its own timeout / TLS / proxy config, so
        /// `.timeout()` does not apply to an injected agent (configure it on the agent); extra
        /// `.request_header()`s are still applied per request.
        #[cfg(feature = "ureq")]
        pub fn ureq_agent(&mut self, agent: ::ureq::Agent) -> &mut Self {
            self.$($path).+.client.agent = Some(agent);
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
    };
    ($t:ty, { $($extra:tt)* }) => {
        impl_update_config_accessors!(@emit (impl crate::update::UpdateConfig for $t), { $($extra)* });
    };
    // Generic form for the custom `AsyncUpdate<S>`: a `where (...)` clause carries the bound.
    ($t:ty, where ( $($bound:tt)* )) => {
        impl_update_config_accessors!(
            @emit (impl<S> crate::update::UpdateConfig for $t where $($bound)*),
            {}
        );
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
        fn progress_template(&self) -> &str {
            &self.common.progress_template
        }
        fn progress_chars(&self) -> &str {
            &self.common.progress_chars
        }
        fn auth_token(&self) -> Option<&str> {
            self.common.auth_token.as_deref()
        }
        #[doc(hidden)]
        fn request_timeout(&self) -> Option<std::time::Duration> {
            self.common.request.timeout
        }
        #[doc(hidden)]
        fn request_headers(&self) -> &crate::http_client::HeaderMap {
            &self.common.request.headers
        }
        #[doc(hidden)]
        fn request_client(&self) -> &crate::http_client::ClientOverride {
            &self.common.request.client
        }
        #[doc(hidden)]
        fn progress_callback(&self) -> Option<std::sync::Arc<crate::DynProgressFn>> {
            self.common.progress_callback.as_ref().map(|c| c.0.clone())
        }
        #[doc(hidden)]
        fn verify_callback(&self) -> Option<std::sync::Arc<crate::DynVerifyFn>> {
            self.common.verify.as_ref().map(|c| c.0.clone())
        }
        #[doc(hidden)]
        fn asset_matcher(&self) -> Option<std::sync::Arc<crate::DynAssetMatcher>> {
            self.common.asset_matcher.as_ref().map(|c| c.0.clone())
        }
        #[doc(hidden)]
        #[cfg(feature = "checksums")]
        fn checksum(&self) -> Option<&crate::Checksum> {
            self.common.checksum.as_ref()
        }
        #[cfg(feature = "signatures")]
        fn verifying_keys(&self) -> &[crate::VerifyingKey] {
            &self.common.verifying_keys
        }
        }
    };
}

/// Emit the backend-independent `UpdateBuilder` setters shared by every backend.
///
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
        #[doc(alias = "target_version_tag")]
        #[doc(alias = "target_version")]
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
        #[doc(alias = "identifier")]
        pub fn asset_identifier(&mut self, identifier: impl Into<String>) -> &mut Self {
            self.common.asset_identifier = Some(identifier.into());
            self
        }

        /// Required. Set the exe's name. Also sets `bin_path_in_archive` if it hasn't already been
        /// set.
        ///
        /// This method will append the platform specific executable file suffix
        /// (see `std::env::consts::EXE_SUFFIX`) to the name if it's missing.
        ///
        /// Order matters only when you also set `bin_path_in_archive`: calling
        /// [`bin_path_in_archive`](Self::bin_path_in_archive) *before* `bin_name` keeps your value
        /// (`bin_name` won't overwrite it); a later `bin_name` call also leaves an already-set path
        /// untouched.
        pub fn bin_name(&mut self, name: impl Into<String>) -> &mut Self {
            let name = name.into();
            let raw_bin_name = format!(
                "{}{}",
                name.trim_end_matches(std::env::consts::EXE_SUFFIX),
                std::env::consts::EXE_SUFFIX
            );
            if self.common.bin_path_in_archive.is_none() {
                self.common.bin_path_in_archive = Some(raw_bin_name.clone());
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
        pub fn bin_path_in_archive(&mut self, bin_path: impl Into<String>) -> &mut Self {
            self.common.bin_path_in_archive = Some(bin_path.into());
            self
        }

        /// Toggle download progress bar, defaults to `off`.
        pub fn show_download_progress(&mut self, show: bool) -> &mut Self {
            self.common.show_download_progress = show;
            self
        }

        /// Set download progress style.
        #[doc(alias = "set_progress_style")]
        pub fn progress_style(
            &mut self,
            progress_template: impl Into<String>,
            progress_chars: impl Into<String>,
        ) -> &mut Self {
            self.common.progress_template = progress_template.into();
            self.common.progress_chars = progress_chars.into();
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

        request_config_setters!(common.request);

        /// Register a callback invoked as the release downloads, with
        /// `(bytes_downloaded_so_far, total_bytes)` (`total_bytes` is `None` when the server
        /// sends no `Content-Length`). Independent of `show_download_progress`; use it to drive
        /// a GUI or structured logging. The callback is `Fn`, so track state via interior
        /// mutability (e.g. an `AtomicU64` or a channel).
        #[doc(alias = "set_progress_callback")]
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
        /// extracted binary; returning `false` aborts the update (nothing is installed), so a bad
        /// release cannot replace a working binary. Typical use: run `new --version` and check it.
        ///
        /// This runs **last** in the verification chain and on the **extracted binary**, not the
        /// downloaded archive. The full order is: [`checksum`](Self::checksum) (digest of the
        /// archive) -> signature ([`verifying_keys`](Self::verifying_keys), over the archive) ->
        /// extract -> `verify_with` (the extracted binary) -> replace. Use `checksum`/`verifying_keys`
        /// to gate the download by content; use `verify_with` to gate it by running the new binary.
        pub fn verify_with(
            &mut self,
            verify: impl Fn(&std::path::Path) -> bool + Send + Sync + 'static,
        ) -> &mut Self {
            self.common.verify = Some(crate::VerifyCallback(std::sync::Arc::new(verify)));
            self
        }

        /// Verify the downloaded artifact against an expected [`Checksum`](crate::Checksum)
        /// (e.g. one published in a `SHA256SUMS` file) before installing it. The algorithm is
        /// chosen by the `Checksum` variant.
        #[cfg(feature = "checksums")]
        #[doc(alias = "verifying_checksum")]
        pub fn checksum(&mut self, checksum: crate::Checksum) -> &mut Self {
            self.common.checksum = Some(checksum);
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

/// Emit the inherent async update methods shared by every backend's `Update` (under the `async`
/// feature). Invoked inside a `#[cfg(feature = "async")] impl Update { … }` block. The `Update`
/// type must implement both `UpdateConfig` and the internal `AsyncFetch`.
#[cfg(feature = "async")]
macro_rules! impl_async_update_methods {
    () => {
        /// Async sibling of `update`: display release info and update the current binary to the
        /// latest release, returning the resulting [`Status`](crate::Status).
        ///
        /// Requires a `tokio` runtime (provided by the caller). The release listing and the
        /// download are async; the extract/replace step is synchronous and runs inline, briefly
        /// blocking the executor — keep that in mind on a small single-threaded runtime.
        pub async fn update_async(&self) -> crate::errors::Result<crate::Status> {
            let current_version = crate::update::UpdateConfig::current_version(self).to_string();
            self.update_extended_async()
                .await
                .map(|s| s.into_status(current_version))
        }

        /// Async sibling of `update_extended`: same as [`update_async`](Self::update_async) but
        /// returns [`UpdateStatus`](crate::update::UpdateStatus).
        pub async fn update_extended_async(
            &self,
        ) -> crate::errors::Result<crate::update::UpdateStatus> {
            crate::update::update_extended_async(self).await
        }

        /// Async sibling of `get_latest_release`: fetch the single newest release from the
        /// backend, as a one-element [`Releases`](crate::update::Releases). Call
        /// `.is_update_available()` / `.latest()` on the result for a lightweight pre-check
        /// without downloading or installing anything.
        pub async fn get_latest_release_async(
            &self,
        ) -> crate::errors::Result<crate::update::Releases> {
            crate::update::AsyncFetch::get_latest_release_async(self).await
        }

        /// Async sibling of `get_latest_releases`: fetch the candidate releases from the backend
        /// as a [`Releases`](crate::update::Releases) (newest-first). Call
        /// `.is_update_available()` on the result for a lightweight "is there anything to do?"
        /// check without downloading or installing anything.
        pub async fn get_latest_releases_async(
            &self,
        ) -> crate::errors::Result<crate::update::Releases> {
            crate::update::AsyncFetch::get_latest_releases_async(self).await
        }

        /// Async sibling of `get_release_version`: fetch the [`Release`](crate::update::Release)
        /// matching the given tag/version from the backend. The tag is used verbatim (including any
        /// leading `v`); a missing tag is reported as an error.
        pub async fn get_release_version_async(
            &self,
            ver: &str,
        ) -> crate::errors::Result<crate::update::Release> {
            crate::update::AsyncFetch::get_release_version_async(self, ver).await
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

/// Helper for formatting `errors::Error`s
macro_rules! format_err {
    ($e_type:expr, $literal:expr) => {
        $e_type(format!($literal))
    };
    ($e_type:expr, $literal:expr, $($arg:expr),*) => {
        $e_type(format!($literal, $($arg),*))
    };
}

/// Helper for formatting `errors::Error`s and returning early
macro_rules! bail {
    ($e_type:expr, $literal:expr) => {
        return Err(format_err!($e_type, $literal))
    };
    ($e_type:expr, $literal:expr, $($arg:expr),*) => {
        return Err(format_err!($e_type, $literal, $($arg),*))
    };
}

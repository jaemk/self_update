use regex::Regex;
use std::borrow::Cow;
use std::env::consts::{ARCH, OS};
use std::fs;
use std::path::PathBuf;

use crate::http_client::{self, header};
use crate::{confirm, errors::*, version, Download, Extract, Move, Status};

/// Release asset information
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ReleaseAsset {
    pub download_url: String,
    pub name: String,
}

impl ReleaseAsset {
    /// Construct a `ReleaseAsset` from its name and download URL.
    ///
    /// Useful when implementing a custom [`ReleaseSource`] (the built-in backends build assets from
    /// their own API responses) or when building a `ReleaseAsset` in your own tests — the type is
    /// `#[non_exhaustive]`, so it can't be built with a struct literal from outside the crate.
    pub fn new(name: impl Into<String>, download_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            download_url: download_url.into(),
        }
    }
}

/// Update status with extended information
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum UpdateStatus {
    /// Crate is up to date
    UpToDate,
    /// Crate was updated to the contained release
    Updated(Release),
}

impl UpdateStatus {
    /// Turn the extended information into the crate's standard `Status` enum
    pub fn into_status(self, current_version: String) -> Status {
        match self {
            UpdateStatus::UpToDate => Status::UpToDate(current_version),
            UpdateStatus::Updated(release) => Status::Updated(release.version),
        }
    }

    /// Returns `true` if `Status::UpToDate`
    pub fn uptodate(&self) -> bool {
        matches!(*self, UpdateStatus::UpToDate)
    }

    /// Returns `true` if `Status::Updated`
    pub fn updated(&self) -> bool {
        !self.uptodate()
    }
}

/// Release information
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Release {
    pub name: String,
    pub version: String,
    pub date: String,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

impl Release {
    /// Check if release has an asset who's name contains the specified `target`
    pub fn has_target_asset(&self, target: &str) -> bool {
        self.assets.iter().any(|asset| asset.name.contains(target))
    }

    /// Return the first `ReleaseAsset` for the current release who's name
    /// contains the specified `target` and possibly `identifier`
    pub fn asset_for(&self, target: &str, identifier: Option<&str>) -> Option<ReleaseAsset> {
        self.assets
            .iter()
            // first look specifically for a target with identifier
            .find(|asset| {
                asset.name.contains(target)
                    && if let Some(i) = identifier {
                        asset.name.contains(i)
                    } else {
                        true
                    }
            })
            // otherwise look for a target for the current arch/os with identifier
            .or_else(|| {
                self.assets.iter().find(|asset| {
                    (asset.name.contains(OS) && asset.name.contains(ARCH))
                        && if let Some(i) = identifier {
                            asset.name.contains(i)
                        } else {
                            true
                        }
                })
            })
            // otherwise just with the identifier if set
            .or_else(|| {
                identifier.and_then(|i| self.assets.iter().find(|asset| asset.name.contains(i)))
            })
            .cloned()
    }

    /// Start building a [`Release`].
    ///
    /// `Release` is `#[non_exhaustive]`, so it can't be built with a struct literal from outside the
    /// crate. Use this builder when implementing a custom [`ReleaseSource`] or constructing a
    /// `Release` in your own tests.
    pub fn builder() -> ReleaseBuilder {
        ReleaseBuilder::default()
    }
}

/// Builder for a [`Release`]. Obtain one via [`Release::builder`].
///
/// Only `version` is required (it drives the version comparison); `name` defaults to the version,
/// `date` defaults to empty, `body` to `None`, and `assets` to whatever was added.
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct ReleaseBuilder {
    name: Option<String>,
    version: Option<String>,
    date: Option<String>,
    body: Option<String>,
    assets: Vec<ReleaseAsset>,
}

impl ReleaseBuilder {
    /// Set the release version (required), e.g. `"1.2.3"`. This is what the updater compares against
    /// the current version, so it should be a bare semver string (no leading `v`).
    pub fn version(&mut self, version: impl Into<String>) -> &mut Self {
        self.version = Some(version.into());
        self
    }

    /// Set the release name/title. Defaults to the version if unset.
    pub fn name(&mut self, name: impl Into<String>) -> &mut Self {
        self.name = Some(name.into());
        self
    }

    /// Set the release date string. Defaults to empty if unset.
    pub fn date(&mut self, date: impl Into<String>) -> &mut Self {
        self.date = Some(date.into());
        self
    }

    /// Set the release body / notes.
    pub fn body(&mut self, body: impl Into<String>) -> &mut Self {
        self.body = Some(body.into());
        self
    }

    /// Add a single downloadable asset.
    pub fn asset(&mut self, asset: ReleaseAsset) -> &mut Self {
        self.assets.push(asset);
        self
    }

    /// Add several downloadable assets.
    pub fn assets(&mut self, assets: impl IntoIterator<Item = ReleaseAsset>) -> &mut Self {
        self.assets.extend(assets);
        self
    }

    /// Validate and build the [`Release`]. Errors if `version` was not set.
    pub fn build(&self) -> Result<Release> {
        let version = self
            .version
            .clone()
            .ok_or_else(|| Error::Config("`version` required".to_string()))?;
        Ok(Release {
            name: self.name.clone().unwrap_or_else(|| version.clone()),
            version,
            date: self.date.clone().unwrap_or_default(),
            body: self.body.clone(),
            assets: self.assets.clone(),
        })
    }
}

/// A source of releases for a custom update backend.
///
/// Implement this to update from a host the built-in backends (`github`, `gitlab`, `gitea`, `s3`)
/// don't cover — a different forge, a private artifact registry, a plain HTTP directory, etc. — and
/// then drive a full update through [`backends::custom`](crate::backends::custom), which reuses the
/// crate's compare → select-asset → download → verify → extract → install orchestration. The trait
/// is **not** sealed, unlike [`ReleaseUpdate`].
///
/// You own *where releases come from*: each method makes whatever HTTP request (with whatever auth,
/// pagination, and parsing) your host needs and returns [`Release`]s built via [`Release::builder`].
/// The crate owns *how the update happens* — asset selection, transport for the download, checksum
/// /signature verification, extraction, and the install — so you do not touch the low-level
/// primitives.
///
/// Implementations must be `Send + Sync`, and the builder stores the source as
/// `impl ReleaseSource + 'static`, so a source that needs to reference outer state should own it
/// (e.g. hold an `Arc<Config>`) rather than borrow it.
///
/// This trait is **synchronous**. For a natively-async source, implement
/// [`AsyncReleaseSource`] and drive it through
/// [`backends::custom::AsyncUpdate`](crate::backends::custom::AsyncUpdate); to reuse a `Clone`
/// sync `ReleaseSource` from the async API, wrap it in
/// [`backends::custom::Blocking`](crate::backends::custom::Blocking).
///
/// On failure, return one of the public [`Error`](crate::errors::Error) variants — they are plain
/// constructible tuple variants, e.g. `Error::Network("…".into())` for a failed request,
/// `Error::Release("…".into())` for a missing/unparseable release, or `Error::Config("…".into())`
/// for a misconfiguration.
pub trait ReleaseSource: Send + Sync {
    /// Fetch the single newest release.
    fn get_latest_release(&self) -> Result<Release>;

    /// Fetch the candidate releases, **newest first**. Return all the releases you want considered;
    /// the updater discards any that are not strictly newer than the current version, prefers the
    /// newest semver-compatible one, and otherwise offers the newest available (flagged
    /// "not compatible"). You therefore do **not** need to filter out the current or older versions
    /// (they are ignored) — but returning them is harmless, and returning the list newest-first
    /// ensures the right release is chosen. `current_version` is passed only so you may bound the
    /// query if it helps (e.g. stop paginating once you pass it).
    fn get_latest_releases(&self, current_version: &str) -> Result<Vec<Release>>;

    /// Fetch the release for an explicit tag/version.
    fn get_release_version(&self, ver: &str) -> Result<Release>;
}

/// An async source of releases for a custom update backend.
///
/// This is the async analog of [`ReleaseSource`]: implement it to update from a host the built-in
/// backends (`github`, `gitlab`, `gitea`, `s3`) don't cover when your listing transport is itself
/// async, and drive a full update through
/// [`backends::custom::AsyncUpdate`](crate::backends::custom::AsyncUpdate), which reuses the
/// crate's compare → select-asset → download → verify → extract → install orchestration.
///
/// You own *where releases come from*: each method makes whatever async HTTP request (with whatever
/// auth, pagination, and parsing) your host needs and returns [`Release`]s built via
/// [`Release::builder`]. The crate owns *how the update happens* — asset selection, the download,
/// checksum/signature verification, extraction, and the install.
///
/// This trait is consumed through generics (the async updater is generic over its source, never a
/// `dyn AsyncReleaseSource`), so the plain `async fn` methods need no boxing — there is no
/// `async-trait` dependency. Implementations must be `Send + Sync`, and the returned futures must
/// be `Send` (they are awaited inside the updater).
///
/// To reuse an existing `Clone` sync [`ReleaseSource`] from the async API, wrap it in
/// [`backends::custom::Blocking`](crate::backends::custom::Blocking), which runs the sync fetches
/// on [`tokio::task::spawn_blocking`].
///
/// On failure, return one of the public [`Error`](crate::errors::Error) variants — they are plain
/// constructible tuple variants, e.g. `Error::Network("…".into())` for a failed request,
/// `Error::Release("…".into())` for a missing/unparseable release, or `Error::Config("…".into())`
/// for a misconfiguration.
#[cfg(feature = "async")]
#[allow(async_fn_in_trait)]
pub trait AsyncReleaseSource: Send + Sync {
    /// Fetch the single newest release.
    async fn get_latest_release(&self) -> Result<Release>;

    /// Fetch the candidate releases, **newest first**. See
    /// [`ReleaseSource::get_latest_releases`] for how the updater treats the returned list.
    async fn get_latest_releases(&self, current_version: &str) -> Result<Vec<Release>>;

    /// Fetch the release for an explicit tag/version.
    async fn get_release_version(&self, ver: &str) -> Result<Release>;
}

/// Internal async counterpart of the three backend fetch methods, implemented by every backend's
/// `Update` (and the custom `AsyncUpdate`) when the `async` feature is on. It is only ever used
/// through generics (the async orchestrator is generic over `UpdateConfig + AsyncFetch`), never as
/// a trait object, so `async fn` in the trait needs no boxing.
#[cfg(feature = "async")]
#[allow(async_fn_in_trait)]
pub(crate) trait AsyncFetch {
    async fn get_latest_release_async(&self) -> Result<Release>;
    async fn get_latest_releases_async(&self, current_version: &str) -> Result<Vec<Release>>;
    async fn get_release_version_async(&self, ver: &str) -> Result<Release>;
}

/// Implementation detail used to seal [`ReleaseUpdate`].
///
/// Downstream code can *use* `ReleaseUpdate` (every backend's `build()` returns a
/// `Box<dyn ReleaseUpdate>`) but cannot implement it for foreign types, which leaves the
/// crate free to evolve the trait without a breaking change.
pub(crate) mod sealed {
    pub trait Sealed {}
}

/// The shared configuration surface of an updater: the accessors every backend's `Update` exposes
/// (current version, target, bin name/path, progress style, auth, transport, verification hooks,
/// …).
///
/// This trait is **sealed**: it is implemented only by this crate's backend `Update` types and the
/// custom updaters, and cannot be implemented for types outside the crate. It is the supertrait of
/// both [`ReleaseUpdate`] (sync) and the orchestrator's async path, so an async-only updater need
/// not implement the sync fetch methods.
///
/// You rarely name this trait directly: accessor calls (e.g. `bin_name()`, `current_version()`,
/// `target()`) resolve on a `dyn ReleaseUpdate` value without importing it. It is only needed in
/// scope (`use self_update::UpdateConfig;`) to call an accessor from generic code bounded
/// `R: ReleaseUpdate`.
pub trait UpdateConfig: sealed::Sealed {
    /// Current version of binary being updated
    fn current_version(&self) -> &str;

    /// Target platform the update is being performed for
    fn target(&self) -> &str;

    /// Release tag optionally specified for the update (set via `release_tag`)
    #[doc(alias = "target_version")]
    #[doc(alias = "target_version_tag")]
    fn release_tag(&self) -> Option<&str>;

    /// Optional identifier for determining the asset among multiple matches (set via
    /// `asset_identifier`)
    #[doc(alias = "identifier")]
    fn asset_identifier(&self) -> Option<&str> {
        None
    }

    /// Name of the binary being updated
    fn bin_name(&self) -> &str;

    /// Installation path for the binary being updated
    fn bin_install_path(&self) -> PathBuf;

    /// Path of the binary to be extracted from release package
    fn bin_path_in_archive(&self) -> &str;

    /// Flag indicating if progress information shall be output when downloading a release
    fn show_download_progress(&self) -> bool;

    /// Flag indicating if process informative messages shall be output
    fn show_output(&self) -> bool;

    /// Flag indicating if the user shouldn't be prompted to confirm an update
    fn no_confirm(&self) -> bool;

    /// Message template to use if `show_download_progress` is set (see `indicatif::ProgressStyle`)
    fn progress_template(&self) -> &str;

    /// Progress characters to use if `show_download_progress` is set (see `indicatif::ProgressStyle`)
    fn progress_chars(&self) -> &str;

    /// Authorisation token for communicating with backend
    fn auth_token(&self) -> Option<&str>;

    /// Per-request timeout to apply to backend HTTP requests, if any.
    #[doc(hidden)]
    fn request_timeout(&self) -> Option<std::time::Duration>;

    /// Extra HTTP headers to merge into every backend request.
    #[doc(hidden)]
    fn request_headers(&self) -> &http_client::HeaderMap;

    /// Optional user-supplied HTTP client to apply to the download, mirroring the listing requests.
    #[doc(hidden)]
    fn request_client(&self) -> &http_client::ClientOverride;

    /// Optional download-progress callback to forward to the download step.
    #[doc(hidden)]
    fn progress_callback(&self) -> Option<std::sync::Arc<crate::DynProgressFn>>;

    /// Optional post-update verification hook, run on the extracted binary before install.
    #[doc(hidden)]
    fn verify_callback(&self) -> Option<std::sync::Arc<crate::DynVerifyFn>>;

    /// Optional custom asset matcher, overriding the built-in target/identifier selection.
    #[doc(hidden)]
    fn asset_matcher(&self) -> Option<std::sync::Arc<crate::DynAssetMatcher>> {
        None
    }

    /// Optional checksum to verify the downloaded artifact against before installing it.
    #[doc(hidden)]
    #[cfg(feature = "checksums")]
    fn checksum(&self) -> Option<&crate::Checksum>;

    /// ed25519ph verifying keys to validate a download's authenticity
    #[cfg(feature = "signatures")]
    fn verifying_keys(&self) -> &[crate::VerifyingKey] {
        &[]
    }

    /// Construct a header with an authorisation entry if an auth token is provided
    fn api_headers(&self, auth_token: Option<&str>) -> Result<http_client::HeaderMap> {
        let mut headers = header::HeaderMap::new();

        if let Some(token) = auth_token {
            let value = format!("token {}", token).parse().map_err(|_| {
                Error::Config(
                    "the auth token contains characters that are not valid in an HTTP \
                     header value"
                        .to_string(),
                )
            })?;
            headers.insert(header::AUTHORIZATION, value);
        };

        Ok(headers)
    }
}

/// Updates to a specified or latest release.
///
/// This trait is **sealed** (via its [`UpdateConfig`] supertrait): it is implemented only by this
/// crate's backend `Update` types and cannot be implemented for types outside the crate. You
/// consume it as the return type of each backend's `build()` (`Box<dyn ReleaseUpdate>`) — call
/// `update()` / `update_extended()` on it — but you do not implement it yourself.
///
/// The shared accessor methods live on the [`UpdateConfig`] supertrait. They resolve on a
/// `dyn ReleaseUpdate` without importing it; bring it into scope (`use self_update::UpdateConfig;`)
/// only to call them from generic code bounded `R: ReleaseUpdate`.
pub trait ReleaseUpdate: UpdateConfig {
    /// Fetch details of the latest release from the backend
    fn get_latest_release(&self) -> Result<Release>;

    /// Fetch details of the latest release from the backend
    fn get_latest_releases(&self, current_version: &str) -> Result<Vec<Release>>;

    /// Fetch details of the release matching the specified version
    fn get_release_version(&self, ver: &str) -> Result<Release>;

    /// Display release information and update the current binary to the latest release, pending
    /// confirmation from the user.
    ///
    /// Returns a [`Status`] carrying only the version tag. Use [`update_extended`](Self::update_extended)
    /// instead if you need the full [`Release`] details (name, date, body, assets) of the installed
    /// release.
    fn update(&self) -> Result<Status> {
        let current_version = self.current_version().to_string();
        self.update_extended()
            .map(|s| s.into_status(current_version))
    }

    /// Same as `update`, but returns `UpdateStatus`.
    fn update_extended(&self) -> Result<UpdateStatus> {
        let current_version = self.current_version();
        let show_output = self.show_output();
        print_check_header(self.target(), current_version, show_output);

        let release = match self.release_tag() {
            None => {
                print_flush(show_output, "Checking latest released version... ")?;
                let releases = self.get_latest_releases(current_version)?;
                match choose_latest_release(releases, current_version, show_output)? {
                    Some(release) => release,
                    None => return Ok(UpdateStatus::UpToDate),
                }
            }
            Some(ref ver) => {
                println(show_output, &format!("Looking for tag: {}", ver));
                self.get_release_version(ver)?
            }
        };

        let target_asset = resolve_and_confirm(self, &release)?;

        let tmp_archive_dir = tempfile::TempDir::new()?;
        let tmp_archive_path = tmp_archive_dir.path().join(&target_asset.name);
        let mut tmp_archive = fs::File::create(&tmp_archive_path)?;

        println(show_output, "Downloading...");
        build_download(self, &target_asset)?.download_to(&mut tmp_archive)?;

        finish_update(self, release, &tmp_archive_dir, &tmp_archive_path)
    }
}

/// Print the "Checking target-arch / current version" header lines.
fn print_check_header(target: &str, current_version: &str, show_output: bool) {
    println(show_output, &format!("Checking target-arch... {}", target));
    println(
        show_output,
        &format!("Checking current version... v{}", current_version),
    );
}

/// Given the releases fetched for the "latest" path, choose the one to update to, printing the
/// usual progress messages. `Ok(None)` means there is nothing newer than the current version
/// (already up to date). Shared by the sync and async orchestrators.
fn choose_latest_release(
    releases: Vec<Release>,
    current_version: &str,
    show_output: bool,
) -> Result<Option<Release>> {
    // Only consider releases strictly newer than the current version. The built-in backends already
    // pre-filter this way, so this is a no-op for them; it matters for `backends::custom`, whose
    // `ReleaseSource` may return the current (or older) releases — without this guard the fallback
    // below would treat the current version as an available "update" and re-install it.
    let mut releases = releases
        .into_iter()
        .filter(|r| version::bump_is_greater(current_version, &r.version).unwrap_or(false))
        .collect::<Vec<_>>();

    // Sort the candidates semver-descending (newest first) so the selection below does not depend
    // on the order the source/backend returned them. The built-in backends already sort or filter,
    // but `backends::custom`'s `ReleaseSource` may hand back releases in any order. Mirrors the
    // descending comparator in `backends::s3::sort_newer`.
    releases.sort_by(
        |x, y| match version::bump_is_greater(&y.version, &x.version) {
            Ok(is_greater) => {
                if is_greater {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            }
            // Ignoring release due to an unexpected failure in parsing its version string
            Err(_) => std::cmp::Ordering::Greater,
        },
    );

    // Filter to versions compatible with the current one.
    let compatible_releases = releases
        .iter()
        .filter(|r| version::bump_is_compatible(current_version, &r.version).unwrap_or(false))
        .collect::<Vec<_>>();

    let release = if let Some(release) = compatible_releases.first().cloned() {
        println(
            show_output,
            &format!(
                "v{} ({} versions compatible)",
                release.version,
                compatible_releases.len()
            ),
        );
        release.clone()
    } else if let Some(release) = releases.first() {
        println(
            show_output,
            &format!(
                "v{} ({} versions available)",
                release.version,
                releases.len()
            ),
        );
        release.clone()
    } else {
        println(show_output, "up-to-date.");
        return Ok(None);
    };

    println(
        show_output,
        &format!(
            "New release found! v{} --> v{}",
            current_version, release.version
        ),
    );
    let qualifier = if version::bump_is_compatible(current_version, &release.version)? {
        ""
    } else {
        "*NOT* "
    };
    println(
        show_output,
        &format!("New release is {}compatible", qualifier),
    );

    Ok(Some(release))
}

/// Select the asset to download (custom matcher or the built-in target/identifier match), print the
/// release status, and prompt for confirmation unless suppressed. Shared by both orchestrators.
fn resolve_and_confirm<U: UpdateConfig + ?Sized>(u: &U, release: &Release) -> Result<ReleaseAsset> {
    let target = u.target();
    let target_asset = match u.asset_matcher() {
        Some(matcher) => matcher(&release.assets),
        None => release.asset_for(target, u.asset_identifier()),
    }
    .ok_or_else(|| format_err!(Error::Release, "No asset found for target: `{}`", target))?;

    let prompt_confirmation = !u.no_confirm();
    if u.show_output() || prompt_confirmation {
        println!("\n{} release status:", u.bin_name());
        println!("  * Current exe: {:?}", u.bin_install_path());
        println!("  * New exe release: {:?}", target_asset.name);
        println!("  * New exe download url: {:?}", target_asset.download_url);
        println!("\nThe new release will be downloaded/extracted and the existing binary will be replaced.");
    }
    if prompt_confirmation {
        confirm("Do you want to continue? [Y/n] ")?;
    }
    Ok(target_asset)
}

/// Build the [`Download`] for an asset, applying the auth/accept/extra headers, timeout, progress
/// callback, and progress style from the updater. Shared by both orchestrators; the caller drives
/// it with `download_to` (sync) or `download_to_async` (async).
fn build_download<U: UpdateConfig + ?Sized>(
    u: &U,
    target_asset: &ReleaseAsset,
) -> Result<Download> {
    let mut download = Download::from_url(&target_asset.download_url);
    let mut headers = u.api_headers(u.auth_token())?;
    headers.insert(header::ACCEPT, "application/octet-stream".parse().unwrap());
    // Apply the user's extra request headers to the download too.
    for (name, value) in u.request_headers() {
        headers.insert(name.clone(), value.clone());
    }
    download.replace_headers(headers);
    // Forward any injected HTTP client so the download reuses it too.
    download.set_client_override(u.request_client().clone());
    if let Some(timeout) = u.request_timeout() {
        download.timeout(timeout);
    }
    if let Some(callback) = u.progress_callback() {
        download.set_progress_callback_arc(callback);
    }
    download.show_download_progress(u.show_download_progress());
    download.progress_style(u.progress_template(), u.progress_chars());
    Ok(download)
}

/// Verify the downloaded archive (checksum/signature), extract the binary, and install it. This is
/// the sync tail shared verbatim by the sync and async update flows. Consumes `release` and returns
/// the resulting status.
fn finish_update<U: UpdateConfig + ?Sized>(
    u: &U,
    release: Release,
    tmp_archive_dir: &tempfile::TempDir,
    tmp_archive_path: &std::path::Path,
) -> Result<UpdateStatus> {
    let show_output = u.show_output();

    #[cfg(feature = "checksums")]
    if let Some(checksum) = u.checksum() {
        checksum.verify(tmp_archive_path)?;
    }

    #[cfg(feature = "signatures")]
    verify_signature(tmp_archive_path, u.verifying_keys())?;

    print_flush(show_output, "Extracting archive... ")?;

    let bin_path_str = Cow::Borrowed(u.bin_path_in_archive());

    // Substitute the `var` variable in a string with the given `val` value.
    // Variable format: `{{ var }}`
    fn substitute<'a: 'b, 'b>(str: &'a str, var: &str, val: &str) -> Cow<'b, str> {
        let format = format!(r"\{{\{{[[:space:]]*{}[[:space:]]*\}}\}}", var);
        Regex::new(&format).unwrap().replace_all(str, val)
    }

    let bin_path_str = substitute(&bin_path_str, "version", &release.version);
    let bin_path_str = substitute(&bin_path_str, "target", u.target());
    let bin_path_str = substitute(&bin_path_str, "bin", u.bin_name());
    let bin_path_str = bin_path_str.as_ref();

    Extract::from_source(tmp_archive_path).extract_file(tmp_archive_dir.path(), bin_path_str)?;
    let new_exe = tmp_archive_dir.path().join(bin_path_str);

    println(show_output, "Done");

    print_flush(show_output, "Replacing binary file... ")?;

    install_binary(
        &new_exe,
        &u.bin_install_path(),
        u.verify_callback().as_deref(),
    )?;
    println(show_output, "Done");

    Ok(UpdateStatus::Updated(release))
}

/// Async sibling of [`ReleaseUpdate::update_extended`]: identical flow with the release listing and
/// the download done asynchronously, reusing the shared sync helpers for selection, confirmation,
/// verification, extraction, and install.
#[cfg(feature = "async")]
pub(crate) async fn update_extended_async<U>(u: &U) -> Result<UpdateStatus>
where
    // `AsyncFetch` is never used through a trait object (the async API hands out a concrete
    // `Update`), so `U` is always `Sized` here — unlike the shared sync helpers above. Only the
    // accessors (`UpdateConfig`) and the async fetches are used, never the sync fetches, so the
    // bound is narrower than `ReleaseUpdate`.
    U: UpdateConfig + AsyncFetch,
{
    let current_version = u.current_version();
    let show_output = u.show_output();
    print_check_header(u.target(), current_version, show_output);

    let release = match u.release_tag() {
        None => {
            print_flush(show_output, "Checking latest released version... ")?;
            let releases = u.get_latest_releases_async(current_version).await?;
            match choose_latest_release(releases, current_version, show_output)? {
                Some(release) => release,
                None => return Ok(UpdateStatus::UpToDate),
            }
        }
        Some(ref ver) => {
            println(show_output, &format!("Looking for tag: {}", ver));
            u.get_release_version_async(ver).await?
        }
    };

    let target_asset = resolve_and_confirm(u, &release)?;

    let tmp_archive_dir = tempfile::TempDir::new()?;
    let tmp_archive_path = tmp_archive_dir.path().join(&target_asset.name);
    let mut tmp_archive = fs::File::create(&tmp_archive_path)?;

    println(show_output, "Downloading...");
    build_download(u, &target_asset)?
        .download_to_async(&mut tmp_archive)
        .await?;

    finish_update(u, release, &tmp_archive_dir, &tmp_archive_path)
}

/// Run the post-update verification hook (if any) on the freshly-extracted binary, then install
/// it — replacing the current executable in place, or moving it to `bin_install_path`. If the
/// hook returns `false` the install is aborted before anything is replaced.
fn install_binary(
    new_exe: &std::path::Path,
    bin_install_path: &std::path::Path,
    verify: Option<&crate::DynVerifyFn>,
) -> Result<()> {
    if let Some(verify) = verify {
        if !verify(new_exe) {
            bail!(
                Error::Update,
                "post-update verification rejected the new binary"
            )
        }
    }
    let current_exe = std::env::current_exe()?;
    if bin_install_path == current_exe.as_path() {
        self_replace::self_replace(new_exe)?;
    } else {
        Move::from_source(new_exe).to_dest(bin_install_path)?;
    }
    Ok(())
}

// Print out message based on provided flag and flush the output buffer
fn print_flush(show_output: bool, msg: &str) -> Result<()> {
    if show_output {
        print_flush!("{}", msg);
    }
    Ok(())
}

// Print out message based on provided flag
fn println(show_output: bool, msg: &str) {
    if show_output {
        println!("{}", msg);
    }
}

#[cfg(feature = "signatures")]
fn verify_signature(
    archive_path: &std::path::Path,
    keys: &[[u8; zipsign_api::PUBLIC_KEY_LENGTH]],
) -> crate::Result<()> {
    if keys.is_empty() {
        return Ok(());
    }

    println!("Verifying downloaded file...");

    let archive_kind = crate::detect_archive(archive_path)?;
    #[cfg(any(feature = "archive-tar", feature = "archive-zip"))]
    {
        let context = archive_path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.as_bytes())
            .ok_or(Error::NonUTF8)?;

        let keys = keys.iter().copied().map(Ok);
        let keys =
            zipsign_api::verify::collect_keys(keys).map_err(zipsign_api::ZipsignError::from)?;

        let mut exe = std::fs::File::open(archive_path)?;

        match archive_kind {
            #[cfg(feature = "archive-tar")]
            crate::ArchiveKind::Tar(Some(crate::Compression::Gz)) => {
                zipsign_api::verify::verify_tar(&mut exe, &keys, Some(context))
                    .map_err(zipsign_api::ZipsignError::from)?;
                return Ok(());
            }
            #[cfg(feature = "archive-zip")]
            crate::ArchiveKind::Zip => {
                zipsign_api::verify::verify_zip(&mut exe, &keys, Some(context))
                    .map_err(zipsign_api::ZipsignError::from)?;
                return Ok(());
            }
            _ => {}
        }
    }
    Err(Error::NoSignatures(archive_kind))
}

#[cfg(test)]
mod tests {
    use super::{choose_latest_release, install_binary};
    use crate::errors::Result;
    use crate::update::Release;
    use crate::DynVerifyFn;

    fn rel(version: &str) -> Release {
        Release::builder().version(version).build().unwrap()
    }

    #[test]
    fn choose_latest_release_up_to_date_when_nothing_newer() {
        // No releases at all.
        assert!(choose_latest_release(vec![], "1.0.0", false)
            .unwrap()
            .is_none());

        // A source (e.g. a custom backend) that returns the current and older versions must be
        // treated as up-to-date — not re-install the current version. (Regression test.)
        let chosen =
            choose_latest_release(vec![rel("1.0.0"), rel("0.9.0")], "1.0.0", false).unwrap();
        assert!(
            chosen.is_none(),
            "current/older releases must not be offered as an update"
        );
    }

    #[test]
    fn choose_latest_release_prefers_newest_compatible() {
        let chosen = choose_latest_release(
            vec![rel("1.2.0"), rel("1.1.0"), rel("1.0.0")],
            "1.0.0",
            false,
        )
        .unwrap()
        .expect("a compatible newer release is chosen");
        assert_eq!(chosen.version, "1.2.0");
    }

    #[test]
    fn choose_latest_release_sorts_out_of_order_candidates() {
        // A source (e.g. a custom backend) that returns candidates in an arbitrary order must still
        // yield the newest compatible release — "newest" must not depend on caller ordering.
        let chosen = choose_latest_release(
            vec![rel("1.1.0"), rel("1.4.2"), rel("1.0.5"), rel("1.3.0")],
            "1.0.0",
            false,
        )
        .unwrap()
        .expect("the newest compatible release is chosen regardless of input order");
        assert_eq!(chosen.version, "1.4.2");

        // Same set, reversed — the choice must be identical.
        let chosen = choose_latest_release(
            vec![rel("1.3.0"), rel("1.0.5"), rel("1.4.2"), rel("1.1.0")],
            "1.0.0",
            false,
        )
        .unwrap()
        .expect("the newest compatible release is chosen regardless of input order");
        assert_eq!(chosen.version, "1.4.2");
    }

    #[test]
    fn choose_latest_release_ignores_unparseable_versions() {
        // An unparseable version is dropped by the leading `bump_is_greater(...).unwrap_or(false)`
        // filter (it never reaches the sort comparator), so a custom source returning junk versions
        // must not crash or be chosen — the newest parseable compatible release wins.
        let chosen = choose_latest_release(
            vec![
                rel("not-a-version"),
                rel("1.2.0"),
                rel("also-bad"),
                rel("1.1.0"),
            ],
            "1.0.0",
            false,
        )
        .unwrap()
        .expect("the newest parseable compatible release is chosen");
        assert_eq!(chosen.version, "1.2.0");

        // Only junk versions -> nothing selectable -> up-to-date.
        assert!(
            choose_latest_release(vec![rel("junk"), rel("garbage")], "1.0.0", false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn choose_latest_release_falls_back_to_incompatible_newer() {
        // Only a major bump is available: newer than current but not semver-compatible. It is still
        // offered (flagged "*NOT* compatible" in the messages), exercising the fallback branch.
        let chosen = choose_latest_release(vec![rel("2.0.0")], "1.0.0", false)
            .unwrap()
            .expect("an incompatible-but-newer release is still offered");
        assert_eq!(chosen.version, "2.0.0");
    }

    // --- Bound-narrowing compile locks (gap #3) -----------------------------------------------
    //
    // The refactor split the accessors onto the `UpdateConfig` supertrait. These items don't run
    // assertions; they exist to *fail to compile* if the trait relationships regress.

    use crate::update::{ReleaseUpdate, UpdateConfig};

    // A generic helper bounded only on `ReleaseUpdate` must still be able to call the accessors
    // that now live on the `UpdateConfig` supertrait — because `ReleaseUpdate: UpdateConfig`. If
    // the supertrait bound were dropped, `bin_name()`/`target()` would not resolve here.
    fn accessor_via_release_update_bound<R: ReleaseUpdate + ?Sized>(r: &R) -> (String, String) {
        (r.bin_name().to_string(), r.target().to_string())
    }

    // Accessors must also resolve on a `&dyn ReleaseUpdate` (trait-object form returned by every
    // backend's `build()`), again only because of the supertrait relationship.
    fn accessor_via_dyn_release_update(r: &dyn ReleaseUpdate) -> String {
        r.current_version().to_string()
    }

    // A helper bounded directly on `UpdateConfig` is the narrower form the async orchestrator
    // uses; it must compile for any `UpdateConfig`, with no `ReleaseUpdate`/fetch requirement.
    fn accessor_via_update_config_bound<U: UpdateConfig + ?Sized>(u: &U) -> String {
        u.bin_name().to_string()
    }

    #[test]
    fn bound_narrowing_helpers_are_exercised() {
        // Drive the compile-locked helpers against a real backend `Update` so they aren't dead
        // code (and so the trait-object path is actually walked at runtime).
        let upd = crate::backends::custom::Update::configure()
            .source(BoundSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .build()
            .unwrap();

        let (bin, target) = accessor_via_release_update_bound(&*upd);
        assert_eq!(bin, "app");
        assert_eq!(target, "x86_64-unknown-linux-gnu");
        assert_eq!(accessor_via_dyn_release_update(&*upd), "1.0.0");
        assert_eq!(accessor_via_update_config_bound(&*upd), "app");
    }

    struct BoundSource;
    impl crate::update::ReleaseSource for BoundSource {
        fn get_latest_release(&self) -> Result<Release> {
            Release::builder().version("1.0.0").build()
        }
        fn get_latest_releases(&self, _c: &str) -> Result<Vec<Release>> {
            Ok(vec![Release::builder().version("1.0.0").build()?])
        }
        fn get_release_version(&self, v: &str) -> Result<Release> {
            Release::builder().version(v).build()
        }
    }

    #[test]
    fn install_binary_aborts_when_verify_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let new_exe = dir.path().join("new");
        std::fs::write(&new_exe, b"new binary").unwrap();
        // A dest that isn't the current exe takes the move path (not self_replace).
        let dest = dir.path().join("installed");

        let reject: Box<DynVerifyFn> = Box::new(|_: &std::path::Path| false);
        let res = install_binary(&new_exe, &dest, Some(&*reject));
        assert!(res.is_err(), "verify=false must abort the install");
        assert!(
            !dest.exists(),
            "nothing is installed when verification fails"
        );
        assert!(new_exe.exists(), "the extracted binary is left untouched");
    }

    #[test]
    fn install_binary_installs_when_verify_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let new_exe = dir.path().join("new");
        std::fs::write(&new_exe, b"new binary").unwrap();
        let dest = dir.path().join("installed");

        let accept: Box<DynVerifyFn> = Box::new(|_: &std::path::Path| true);
        install_binary(&new_exe, &dest, Some(&*accept)).unwrap();
        assert!(
            dest.exists(),
            "binary is installed when verification passes"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"new binary");
    }

    // Build a custom-backend `Update` carrying `checksum`, to drive `finish_update` directly.
    #[cfg(feature = "checksums")]
    fn update_with_checksum(checksum: crate::Checksum) -> Box<dyn ReleaseUpdate> {
        crate::backends::custom::Update::configure()
            .source(BoundSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .checksum(checksum)
            .build()
            .unwrap()
    }

    // A configured checksum is actually consulted by `finish_update`: a mismatch aborts the
    // update at the checksum gate, before any extraction or install. If the checksum block were
    // dropped, the bogus archive would instead fail later with a non-checksum error and this
    // test would catch it.
    #[cfg(feature = "checksums")]
    #[test]
    fn finish_update_rejects_a_mismatched_checksum_before_extracting() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.tar.gz");
        std::fs::write(&archive_path, b"hello").unwrap();

        // A valid-length but wrong SHA-256 digest (the file's real digest is 2cf24dba…).
        let upd = update_with_checksum(crate::Checksum::Sha256("00".repeat(32)));
        let release = Release::builder().version("1.2.3").build().unwrap();

        let err = super::finish_update(&*upd, release, &dir, &archive_path)
            .expect_err("a mismatched checksum must abort the update");
        let msg = err.to_string();
        assert!(
            msg.contains("checksum mismatch"),
            "expected a checksum-mismatch abort, got: {}",
            msg
        );
    }

    // The complement: a *matching* checksum passes the gate, so the flow proceeds past it. Here
    // the bogus `.tar.gz` then fails at extraction — a different error — proving the checksum was
    // accepted rather than the update being aborted at the gate. Gated additionally on the
    // archive features so the post-gate extraction failure is deterministic.
    #[cfg(all(
        feature = "checksums",
        feature = "archive-tar",
        feature = "compression-flate2"
    ))]
    #[test]
    fn finish_update_passes_a_matching_checksum_then_proceeds() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.tar.gz");
        std::fs::write(&archive_path, b"hello").unwrap();

        // The real SHA-256 of b"hello".
        let digest = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let upd = update_with_checksum(crate::Checksum::Sha256(digest.to_string()));
        let release = Release::builder().version("1.2.3").build().unwrap();

        let err = super::finish_update(&*upd, release, &dir, &archive_path)
            .expect_err("the bytes are not a real archive, so extraction must fail");
        let msg = err.to_string();
        assert!(
            !msg.contains("checksum mismatch"),
            "a matching checksum must pass the gate; the failure should come from extraction, \
             got: {}",
            msg
        );
    }
}

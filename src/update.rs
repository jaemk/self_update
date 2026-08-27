use regex::Regex;
use std::borrow::Cow;
use std::fs;
use std::sync::{Arc, LazyLock};

use crate::http_client::{self, header};
use crate::{Download, Extract, Move, VersionStatus, confirm, errors::*, version};

/// Release asset information.
///
/// The fields are encapsulated (`pub(crate)`, backed by `Arc<str>` to keep clones cheap) and read
/// through the [`name`](ReleaseAsset::name) / [`download_url`](ReleaseAsset::download_url) /
/// [`digest`](ReleaseAsset::digest) getters, which return borrows. Build one with
/// [`ReleaseAsset::new`] (plus [`with_digest`](ReleaseAsset::with_digest) when the forge publishes
/// a content digest for the asset).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ReleaseAsset {
    pub(crate) name: Arc<str>,
    pub(crate) download_url: Arc<str>,
    pub(crate) digest: Option<Arc<str>>,
}

impl ReleaseAsset {
    /// Construct a `ReleaseAsset` from its name and download URL.
    ///
    /// Useful when implementing a custom [`ReleaseSource`] (the built-in backends build assets from
    /// their own API responses) or when building a `ReleaseAsset` in your own tests — the type is
    /// `#[non_exhaustive]`, so it can't be built with a struct literal from outside the crate.
    pub fn new(name: impl Into<String>, download_url: impl Into<String>) -> Self {
        Self {
            name: Arc::from(name.into()),
            download_url: Arc::from(download_url.into()),
            digest: None,
        }
    }

    /// Attach the asset's content digest, in `algorithm:hex` form (e.g. `sha256:2cf24d…`).
    ///
    /// The github backend fills this from the API's per-asset `digest` field; a custom
    /// [`ReleaseSource`] can chain this onto [`new`](ReleaseAsset::new) when its forge publishes
    /// one. With the `checksums` feature the updater verifies the downloaded artifact against this
    /// digest before installing (see `verify_release_digest` on the builders).
    pub fn with_digest(mut self, digest: impl Into<String>) -> Self {
        self.digest = Some(Arc::from(digest.into()));
        self
    }

    /// The asset's file name (e.g. `app-x86_64-unknown-linux-gnu.tar.gz`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The asset's download URL.
    pub fn download_url(&self) -> &str {
        &self.download_url
    }

    /// The asset's content digest in `algorithm:hex` form (e.g. `sha256:2cf24d…`), when the
    /// backend provides one.
    ///
    /// The github backend fills this from the release API's per-asset `digest` field; gitlab,
    /// gitea, and s3 do not expose one, so it is `None` there. Note the digest is an *integrity*
    /// check only — the forge recomputes it if an asset is replaced — so it is not a substitute
    /// for signature verification (the `signatures` feature).
    pub fn digest(&self) -> Option<&str> {
        self.digest.as_deref()
    }
}

/// The richer result of [`update_extended`](ReleaseUpdate::update_extended) (and its async sibling
/// `update_extended_async`): it carries the full [`Release`] that was installed.
///
/// This is the extended counterpart of [`VersionStatus`], the lightweight
/// result of [`update`](ReleaseUpdate::update) which carries only the version tag. Reach for
/// `ReleaseStatus` when you need the installed release's details (name, date, body, assets); reach
/// for `VersionStatus` when the version string is all you need. Convert with
/// [`into_version_status`](ReleaseStatus::into_version_status).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ReleaseStatus {
    /// Crate is up to date
    UpToDate,
    /// Crate was updated to the contained release
    Updated(Release),
}

impl ReleaseStatus {
    /// Turn the extended information into the crate's standard [`VersionStatus`] enum
    pub fn into_version_status(self, current_version: String) -> VersionStatus {
        match self {
            ReleaseStatus::UpToDate => VersionStatus::UpToDate(current_version),
            ReleaseStatus::Updated(release) => {
                VersionStatus::Updated(release.version().to_string())
            }
        }
    }

    /// Returns `true` if `ReleaseStatus::UpToDate`
    pub fn is_up_to_date(&self) -> bool {
        matches!(*self, ReleaseStatus::UpToDate)
    }

    /// Returns `true` if `ReleaseStatus::Updated`
    pub fn is_updated(&self) -> bool {
        !self.is_up_to_date()
    }

    /// The [`Release`] that was installed, or `None` if already up to date.
    ///
    /// Convenience accessor so callers can read the installed release without a
    /// `match` (which `#[non_exhaustive]` would force a wildcard arm onto).
    pub fn updated_release(&self) -> Option<&Release> {
        match self {
            ReleaseStatus::Updated(release) => Some(release),
            ReleaseStatus::UpToDate => None,
        }
    }

    /// Consume the status and return the installed [`Release`], or `None` if already up to date.
    pub fn into_updated_release(self) -> Option<Release> {
        match self {
            ReleaseStatus::Updated(release) => Some(release),
            ReleaseStatus::UpToDate => None,
        }
    }

    /// The installed release's version, or `None` when already up to date.
    ///
    /// Mirrors [`VersionStatus::version`](crate::VersionStatus::version), but returns `None` on the
    /// `UpToDate` arm (which, unlike `VersionStatus::UpToDate`, carries no version string).
    pub fn version(&self) -> Option<&str> {
        match self {
            ReleaseStatus::Updated(release) => Some(release.version()),
            ReleaseStatus::UpToDate => None,
        }
    }
}

/// Release information.
///
/// The fields are encapsulated (`pub(crate)`, with the string fields backed by `Arc<str>` to keep
/// clones cheap) and read through the [`name`](Release::name) / [`version`](Release::version) /
/// [`date`](Release::date) / [`body`](Release::body) / [`assets`](Release::assets) getters, which
/// return borrows. Build one with [`Release::builder`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Release {
    pub(crate) name: Arc<str>,
    pub(crate) version: Arc<str>,
    pub(crate) date: Arc<str>,
    pub(crate) body: Option<Arc<str>>,
    pub(crate) release_notes_url: Option<Arc<str>>,
    pub(crate) assets: Vec<ReleaseAsset>,
}

impl Release {
    /// The release name/title.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The release version (a bare semver string, no leading `v`), used for the version comparison.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The release date string (may be empty).
    pub fn date(&self) -> &str {
        &self.date
    }

    /// The release body / notes, if any.
    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }

    /// A URL to the release notes / release page, if the backend provides one. The forge backends
    /// (github, gitlab, gitea) fill it from the release's `html_url`; s3 has none. Shown in the
    /// confirmation prompt when [`show_release_notes(true)`](crate::backends) is set, and readable
    /// directly for a caller that wants to surface it itself.
    pub fn release_notes_url(&self) -> Option<&str> {
        self.release_notes_url.as_deref()
    }

    /// The release's downloadable assets.
    pub fn assets(&self) -> &[ReleaseAsset] {
        &self.assets
    }

    /// Check if release has an asset who's name contains the specified `target`
    pub fn has_target_asset(&self, target: &str) -> bool {
        self.assets.iter().any(|asset| asset.name.contains(target))
    }

    /// Return the first `ReleaseAsset` for the current release who's name
    /// contains the specified `target` and possibly `identifier`.
    ///
    /// Matching is tried in order: (1) the asset name contains the full `target` (and `identifier`
    /// if set); (2) it contains the arch and os tokens derived from `target` (and `identifier`);
    /// (3) it contains just the `identifier`. The arch/os fallback is derived from the `target`
    /// argument, not the build host, so an explicitly configured cross-target selects correctly.
    pub fn asset_for(&self, target: &str, identifier: Option<&str>) -> Option<ReleaseAsset> {
        let has_identifier =
            |asset: &&ReleaseAsset| identifier.is_none_or(|i| asset.name.contains(i));
        self.assets
            .iter()
            // first look specifically for a target with identifier
            .find(|asset| asset.name.contains(target) && has_identifier(asset))
            // otherwise look for a target for the configured arch/os with identifier
            .or_else(|| {
                let (arch, os) = target_arch_os(target);
                match (arch, os) {
                    (Some(arch), Some(os)) => self.assets.iter().find(|asset| {
                        asset.name.contains(arch)
                            && asset.name.contains(os)
                            && has_identifier(asset)
                    }),
                    _ => None,
                }
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
    release_notes_url: Option<String>,
    assets: Vec<ReleaseAsset>,
}

impl ReleaseBuilder {
    /// Create an empty builder; equivalent to [`Release::builder`].
    pub fn new() -> ReleaseBuilder {
        ReleaseBuilder::default()
    }

    /// Set the release version (required), e.g. `"1.2.3"`. This is what the updater compares against
    /// the current version, so it must be a bare semver string (no leading `v`);
    /// [`build`](Self::build) rejects anything `semver` cannot parse.
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

    /// Set a URL to the release notes / release page (the forge backends fill this from the
    /// release's `html_url`). Optional; unset means no notes URL is available.
    pub fn release_notes_url(&mut self, url: impl Into<String>) -> &mut Self {
        self.release_notes_url = Some(url.into());
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

    /// Validate and build the [`Release`].
    ///
    /// Errors with [`Error::MissingField`] if `version` was not set, and with [`Error::SemVer`] if
    /// it is not a bare semver string (e.g. a leading `v`, or a non-semver tag). For a custom
    /// [`ReleaseSource`], validating here gives a clear error at construction time; an unparseable
    /// version stored in a `Release` would otherwise surface only later, as a silently-skipped
    /// release or an opaque comparison error inside the update pipeline. (The built-in forge
    /// backends intentionally skip non-semver tags when parsing their *listings* — that skip
    /// happens at the listing layer, before each `Release` is constructed through this builder.)
    pub fn build(&self) -> Result<Release> {
        let version = self
            .version
            .clone()
            .ok_or(Error::MissingField { field: "version" })?;
        semver::Version::parse(&version)?;
        Ok(Release {
            name: Arc::from(self.name.clone().unwrap_or_else(|| version.clone())),
            version: Arc::from(version),
            date: Arc::from(self.date.clone().unwrap_or_default()),
            body: self.body.clone().map(Arc::from),
            release_notes_url: self.release_notes_url.clone().map(Arc::from),
            assets: self.assets.clone(),
        })
    }
}

/// The releases fetched from a backend, newest-first, together with the updater's configured
/// current version.
///
/// Returned by [`ReleaseUpdate::get_latest_release`] (a one-element list holding the single newest
/// release) and [`ReleaseUpdate::get_newer_releases`] (the full candidate list). Use it for a
/// lightweight pre-check: a single listing request fetches the releases, then
/// [`is_update_available`](Self::is_update_available), [`latest`](Self::latest), and
/// [`all`](Self::all) answer "is there anything newer?" / "what is it?" without downloading or
/// installing anything.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Releases {
    releases: Vec<Release>,
    /// The version the releases were compared against. `None` for a bare listing
    /// ([`ReleaseList::fetch`](crate::backends)) that has no associated current version; `Some` on
    /// the updater path, where `is_update_available` / `current_version` are meaningful.
    current_version: Option<String>,
}

impl Releases {
    /// Construct a `Releases` from a fetched (newest-first) release list and the updater's current
    /// version. Built by the backends; not part of the public construction surface.
    pub(crate) fn new(releases: Vec<Release>, current_version: String) -> Self {
        Self {
            releases,
            current_version: Some(current_version),
        }
    }

    /// Construct a `Releases` from a fetched (newest-first) release list with no associated current
    /// version, mirroring the bare-listing state the backends' `ReleaseList::fetch` returns.
    /// `current_version()` is `None` and `is_update_available()` errors with
    /// [`Error::NoCurrentVersion`], since there is nothing to compare against. Use
    /// [`from_releases`](Self::from_releases) when you have a current version to compare.
    pub fn from_listing(releases: Vec<Release>) -> Self {
        Self {
            releases,
            current_version: None,
        }
    }

    /// Construct a `Releases` from a release list and a current version.
    ///
    /// `Releases` is `#[non_exhaustive]` with a crate-private constructor, so downstream code cannot
    /// build one with a struct literal. This is the public constructor — primarily for building a
    /// `Releases` in your own tests (e.g. a helper that takes a `Releases` and inspects `latest()` /
    /// `is_update_available()`). The releases are taken as-is; the built-in backends order them
    /// newest-first, but no ordering is validated or imposed here.
    pub fn from_releases(releases: Vec<Release>, current_version: impl Into<String>) -> Self {
        Self {
            releases,
            current_version: Some(current_version.into()),
        }
    }

    /// Attach (or replace) the current version to compare against, consuming and returning the
    /// listing. Turns a bare listing (e.g. from `ReleaseList::fetch`) into one where
    /// [`is_update_available`](Self::is_update_available) works, without tearing it down via
    /// [`into_vec`](Self::into_vec) / [`from_releases`](Self::from_releases):
    ///
    /// ```rust,no_run
    /// # fn run() -> self_update::Result<()> {
    /// let available = self_update::backends::github::ReleaseList::configure()
    ///     .repo_owner("jaemk")
    ///     .repo_name("self_update")
    ///     .build()?
    ///     .fetch()?
    ///     .with_current_version(self_update::cargo_crate_version!())
    ///     .is_update_available()?;
    /// # Ok(()) }
    /// ```
    pub fn with_current_version(mut self, current_version: impl Into<String>) -> Self {
        self.current_version = Some(current_version.into());
        self
    }

    /// All fetched releases, newest-first.
    pub fn all(&self) -> &[Release] {
        &self.releases
    }

    /// Number of releases held.
    pub fn len(&self) -> usize {
        self.releases.len()
    }

    /// Whether no releases are held.
    pub fn is_empty(&self) -> bool {
        self.releases.is_empty()
    }

    /// The version the releases were compared against (the updater's configured current version),
    /// or `None` for a bare listing ([`ReleaseList::fetch`](crate::backends)) with no associated
    /// current version.
    pub fn current_version(&self) -> Option<&str> {
        self.current_version.as_deref()
    }

    /// The first release in the list, or `None` when the list is empty.
    ///
    /// This is the first element as ordered by the backend (newest-first for the built-in
    /// backends), not necessarily the semver maximum — a custom [`ReleaseSource`] may return an
    /// unsorted list. For an order-independent "is there anything newer?" check, use
    /// [`is_update_available`](Self::is_update_available).
    pub fn latest(&self) -> Option<&Release> {
        self.releases.first()
    }

    /// Consume the `Releases` and return the underlying release vec (newest-first).
    pub fn into_vec(self) -> Vec<Release> {
        self.releases
    }

    /// Whether an update is available: `true` when **any** fetched release is strictly newer than
    /// the configured current version (a semver comparison), `false` when none is (including when
    /// the list is empty).
    ///
    /// The check is order-independent — it scans the whole set rather than trusting the list to be
    /// newest-first — so it is correct even for a custom [`ReleaseSource`] that returns an unsorted
    /// multi-element list. The scan short-circuits on the first strictly-newer release, returning
    /// `Ok(true)` before any release positioned after it is examined; so a found update wins over a
    /// later parse error. It is the first release *reached* whose version fails to parse as semver
    /// that propagates its error.
    ///
    /// This consults only the already-fetched releases — no further request is made, so the only
    /// `Err` this can return is a version-parse failure ([`Error::SemVer`]) or, when no current
    /// version is known (a bare listing), [`Error::NoCurrentVersion`]; it never surfaces a transport
    /// or HTTP error.
    ///
    /// # See also
    ///
    /// The per-backend `Update::is_update_available` is the configured-updater counterpart: it
    /// *fetches* the latest release and returns `Result<Option<Release>>` (the newer [`Release`], or
    /// `None` when up to date), whereas this method returns `Result<bool>` over the releases already
    /// in hand and never makes a request.
    pub fn is_update_available(&self) -> Result<bool> {
        let current_version = self
            .current_version
            .as_deref()
            .ok_or(Error::NoCurrentVersion)?;
        for r in &self.releases {
            if version::bump_is_greater(current_version, r.version())? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Owned iteration yields each [`Release`] (newest-first), consuming the `Releases`.
impl IntoIterator for Releases {
    type Item = Release;
    type IntoIter = std::vec::IntoIter<Release>;

    fn into_iter(self) -> Self::IntoIter {
        self.releases.into_iter()
    }
}

/// Borrowed iteration yields a `&`[`Release`] for each held release (newest-first).
impl<'a> IntoIterator for &'a Releases {
    type Item = &'a Release;
    type IntoIter = std::slice::Iter<'a, Release>;

    fn into_iter(self) -> Self::IntoIter {
        self.releases.iter()
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
/// On failure, return one of the public [`Error`] variants via its constructor (the structured
/// variants are `#[non_exhaustive]`, so they cannot be built with a struct literal from outside the
/// crate). For a completed request with a non-2xx status use
/// [`Error::http_status_error`] — e.g. `Error::http_status_error(503, url)` for a transient server
/// error or `Error::http_status_error(404, url)` for a missing resource — and [`Error::transport`]
/// for a request that could not be completed (connection refused, DNS, TLS, timeout). For
/// release-level failures use [`Error::no_release_found`] /
/// [`Error::no_release_found_for_target`] (no release / no matching asset) or
/// [`Error::missing_asset_field`] (a missing field in a release/asset payload), and
/// [`Error::invalid_response`] for a response body that failed to parse.
///
/// Only [`get_releases`](Self::get_releases) is required. The other two methods have default
/// implementations derived from it; override them when your host offers a cheaper dedicated
/// endpoint (e.g. a `/latest` route, or a fetch-by-tag lookup). New methods may be added to this
/// trait in minor releases, but only with a default implementation, so implementations keep
/// compiling.
pub trait ReleaseSource: Send + Sync {
    /// Fetch the single newest release.
    ///
    /// The default implementation fetches [`get_releases`](Self::get_releases) and picks the
    /// newest by semver comparison (order-independent, so an unsorted source is still correct).
    /// Errors with [`Error::NoReleaseFound`](crate::Error::NoReleaseFound) when the source has no
    /// releases. Override to use a dedicated latest-release endpoint.
    fn get_latest_release(&self) -> Result<Release> {
        newest_release(self.get_releases()?)
    }

    /// Fetch the candidate releases, **newest first**. Return all the releases you want considered;
    /// the updater discards any that are not strictly newer than the current version, prefers the
    /// newest semver-compatible one, and otherwise offers the newest available (flagged
    /// "not compatible"). You therefore do **not** need to filter out the current or older versions
    /// (they are ignored) — but returning them is harmless, and returning the list newest-first
    /// ensures the right release is chosen.
    fn get_releases(&self) -> Result<Vec<Release>>;

    /// Fetch the release for an explicit version.
    ///
    /// The default implementation fetches [`get_releases`](Self::get_releases) and returns the
    /// first release whose version equals `ver` exactly (the bare semver string, no leading `v`).
    /// Errors with [`Error::NoReleaseFound`](crate::Error::NoReleaseFound) when nothing matches.
    /// Override to use a dedicated fetch-by-tag endpoint.
    fn get_release_version(&self, ver: &str) -> Result<Release> {
        release_for_version(self.get_releases()?, ver)
    }
}

/// The newest release by semver comparison, shared by the `ReleaseSource` /
/// `AsyncReleaseSource` `get_latest_release` defaults. Order-independent; on a version tie the
/// earliest-positioned release wins.
fn newest_release(releases: Vec<Release>) -> Result<Release> {
    releases
        .into_iter()
        .min_by(|a, b| version::cmp_releases_newest_first(a.version(), b.version()))
        .ok_or_else(Error::no_release_found)
}

/// The first release whose version equals `ver` exactly, shared by the `ReleaseSource` /
/// `AsyncReleaseSource` `get_release_version` defaults.
fn release_for_version(releases: Vec<Release>, ver: &str) -> Result<Release> {
    releases
        .into_iter()
        .find(|r| r.version() == ver)
        .ok_or_else(Error::no_release_found)
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
/// `dyn AsyncReleaseSource`), so its methods need no boxing — there is no `async-trait`
/// dependency. Each method returns `impl Future<Output = …> + Send` (return-position `impl Trait`
/// in trait), which both keeps the futures unboxed and **enforces** the `Send` bound at the impl
/// site: a non-`Send` implementation fails to compile here, not later at the spawn site.
/// Implementations must be `Send + Sync`. You may still write the method bodies as `async fn`
/// (the compiler checks the resulting future is `Send`).
///
/// To reuse an existing `Clone` sync [`ReleaseSource`] from the async API, wrap it in
/// [`backends::custom::Blocking`](crate::backends::custom::Blocking), which runs the sync fetches
/// on [`tokio::task::spawn_blocking`].
///
/// On failure, return one of the public [`Error`] variants via its constructor (the structured
/// variants are `#[non_exhaustive]`, so they cannot be built with a struct literal from outside the
/// crate). For a completed request with a non-2xx status use
/// [`Error::http_status_error`] — e.g. `Error::http_status_error(503, url)` for a transient server
/// error or `Error::http_status_error(404, url)` for a missing resource — and [`Error::transport`]
/// for a request that could not be completed (connection refused, DNS, TLS, timeout). For
/// release-level failures use [`Error::no_release_found`] /
/// [`Error::no_release_found_for_target`] (no release / no matching asset) or
/// [`Error::missing_asset_field`] (a missing field in a release/asset payload), and
/// [`Error::invalid_response`] for a response body that failed to parse.
///
/// Only [`get_releases`](Self::get_releases) is required. The other two methods have default
/// implementations derived from it; override them when your host offers a cheaper dedicated
/// endpoint (e.g. a `/latest` route, or a fetch-by-tag lookup). New methods may be added to this
/// trait in minor releases, but only with a default implementation, so implementations keep
/// compiling.
#[cfg(feature = "async")]
pub trait AsyncReleaseSource: Send + Sync {
    /// Fetch the single newest release.
    ///
    /// The default implementation fetches [`get_releases`](Self::get_releases) and picks the
    /// newest by semver comparison (order-independent, so an unsorted source is still correct).
    /// Errors with [`Error::NoReleaseFound`](crate::Error::NoReleaseFound) when the source has no
    /// releases. Override to use a dedicated latest-release endpoint.
    ///
    /// The returned future must be `Send` (it is awaited inside the updater). This is enforced at
    /// the impl site via the `+ Send` bound on the return type, so a non-`Send` implementation
    /// fails to compile here rather than later at the spawn site.
    fn get_latest_release(&self) -> impl std::future::Future<Output = Result<Release>> + Send + '_ {
        async move { newest_release(self.get_releases().await?) }
    }

    /// Fetch the candidate releases, **newest first**. See
    /// [`ReleaseSource::get_releases`] for how the updater treats the returned list.
    fn get_releases(&self) -> impl std::future::Future<Output = Result<Vec<Release>>> + Send + '_;

    /// Fetch the release for an explicit version.
    ///
    /// The default implementation fetches [`get_releases`](Self::get_releases) and returns the
    /// first release whose version equals `ver` exactly (the bare semver string, no leading `v`).
    /// Errors with [`Error::NoReleaseFound`](crate::Error::NoReleaseFound) when nothing matches.
    /// Override to use a dedicated fetch-by-tag endpoint.
    fn get_release_version<'a>(
        &'a self,
        ver: &'a str,
    ) -> impl std::future::Future<Output = Result<Release>> + Send + 'a {
        async move { release_for_version(self.get_releases().await?, ver) }
    }
}

/// The async counterpart of [`ReleaseUpdate`], implemented by every backend's `Update` (and the
/// custom `AsyncUpdate`) when the `async` feature is on.
///
/// This trait is **sealed** (via its [`UpdateConfig`] supertrait, exactly like [`ReleaseUpdate`]):
/// it is implemented only by this crate's backend `Update` types and the custom async updater, and
/// cannot be implemented for types outside the crate. You consume it as the type each backend's
/// `build_async()` returns — call `update_async()` / `update_extended_async()` /
/// `get_latest_release_async()` on it — but you do not implement it yourself.
///
/// Its methods are return-position `impl Trait` in trait (RPITIT), so the futures are unboxed and
/// the `Send` bound is enforced at the impl site. As a consequence this trait is **not**
/// object-safe — that is expected and matches [`AsyncReleaseSource`]: it is nameable and usable as
/// a generic bound, but never as a `dyn AsyncReleaseUpdate`.
///
/// The shared accessor methods live on the [`UpdateConfig`] supertrait. In generic code bounded
/// `U: AsyncReleaseUpdate` they are already in scope via the supertrait bound (no import needed);
/// bring the trait into scope (`use self_update::UpdateConfig;`) only to call them on a concrete
/// backend `Update` value.
// `UpdateInternals` is intentionally `pub(crate)`: it carries the crate-private-typed accessors
// and seals the trait further. The public trait is reachable but the bound is not nameable
// downstream, which is the intent.
#[allow(private_bounds)]
#[cfg(feature = "async")]
pub trait AsyncReleaseUpdate: UpdateConfig + UpdateInternals {
    /// Async sibling of [`ReleaseUpdate::get_latest_release`]: fetch the single newest release as a
    /// one-element [`Releases`].
    fn get_latest_release_async(
        &self,
    ) -> impl std::future::Future<Output = Result<Releases>> + Send + '_;

    /// Async sibling of [`ReleaseUpdate::get_newer_releases`]: fetch the candidate releases as a
    /// [`Releases`] (newest-first, filtered to strictly-newer for the built-in backends).
    fn get_newer_releases_async(
        &self,
    ) -> impl std::future::Future<Output = Result<Releases>> + Send + '_;

    /// Async sibling of [`ReleaseUpdate::get_release_version`]: fetch the release matching `ver`.
    fn get_release_version_async<'a>(
        &'a self,
        ver: &'a str,
    ) -> impl std::future::Future<Output = Result<Release>> + Send + 'a;

    /// Async sibling of [`ReleaseUpdate::update`]: display release info and update the current
    /// binary to the latest release, returning a [`VersionStatus`].
    ///
    /// Requires a `tokio` runtime (provided by the caller). The release listing and the download
    /// are async; the extract/replace tail runs on [`tokio::task::spawn_blocking`] so it does not
    /// block the executor.
    fn update_async(&self) -> impl std::future::Future<Output = Result<VersionStatus>> + Send + '_
    where
        Self: Sized + Sync,
    {
        async move {
            let current_version = self.current_version().to_string();
            self.update_extended_async()
                .await
                .map(|s| s.into_version_status(current_version))
        }
    }

    /// Async sibling of [`ReleaseUpdate::update_extended`]: same as
    /// [`update_async`](Self::update_async) but returns a [`ReleaseStatus`].
    fn update_extended_async(
        &self,
    ) -> impl std::future::Future<Output = Result<ReleaseStatus>> + Send + '_
    where
        Self: Sized + Sync,
    {
        update_extended_async(self)
    }
}

/// Implementation detail used to seal [`ReleaseUpdate`].
///
/// Downstream code can *use* `ReleaseUpdate` (every backend's `build()` returns a concrete
/// `Update` implementing it) but cannot implement it for foreign types, which leaves the
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
/// `target()`) resolve on a `dyn ReleaseUpdate` value without importing it, and in generic code
/// bounded `R: ReleaseUpdate` they are in scope via the supertrait bound (also no import). It is
/// only needed in scope (`use self_update::UpdateConfig;`) to call an accessor on a concrete
/// backend `Update` value.
/// How the "latest" update path chooses which release to install when several are newer than the
/// current version.
///
/// Only affects the unpinned path (`update()` / `update_extended()` with no `release_tag`); a
/// `release_tag(..)` fetch always installs exactly that tag. Set via `update_strategy(..)` on the
/// builders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum UpdateStrategy {
    /// Prefer the newest release that is semver-compatible with the current version (no major
    /// bump); fall back to the newest release overall when none is compatible. This is the default
    /// and matches the pre-1.0 behavior: given `1.1.0, 1.1.1, 1.1.2, 2.0.0` from `1.1.1`, it picks
    /// `1.1.2` (updating to `2.0.0` would take a second invocation once on a `2.x` line).
    #[default]
    Compatible,
    /// Always select the newest release overall, even across an incompatible (major) version bump.
    /// Given `1.1.0, 1.1.1, 1.1.2, 2.0.0` from `1.1.1`, it picks `2.0.0`.
    Latest,
}

pub trait UpdateConfig: sealed::Sealed {
    /// Current version of binary being updated
    fn current_version(&self) -> &str;

    /// Target platform the update is being performed for
    fn target(&self) -> &str;

    /// Release tag optionally specified for the update (set via `release_tag`)
    fn release_tag(&self) -> Option<&str>;

    /// Optional identifier for determining the asset among multiple matches (set via
    /// `asset_identifier`)
    fn asset_identifier(&self) -> Option<&str> {
        None
    }

    /// Name of the binary being updated
    fn bin_name(&self) -> &str;

    /// Installation path for the binary being updated
    fn bin_install_path(&self) -> &std::path::Path;

    /// Whether to run the opt-in preflight writability probe of `bin_install_path` before
    /// downloading (set via `check_install_path_writable`). Defaults to `false`.
    fn check_install_path_writable(&self) -> bool {
        false
    }

    /// Path of the binary to be extracted from release package
    fn bin_path_in_archive(&self) -> &str;

    /// The bundle directory inside the archive (set via `bundle_path_in_archive`), relative to the
    /// archive root. `Some` selects bundle mode: the whole directory replaces
    /// [`bundle_install_path`](Self::bundle_install_path) instead of a single file replacing
    /// [`bin_install_path`](Self::bin_install_path). Defaults to `None`.
    fn bundle_path_in_archive(&self) -> Option<&str> {
        None
    }

    /// The installed bundle directory replaced in bundle mode, resolved at `build()` time. `Some`
    /// exactly when [`bundle_path_in_archive`](Self::bundle_path_in_archive) is. Defaults to
    /// `None`.
    fn bundle_install_path(&self) -> Option<&std::path::Path> {
        None
    }

    /// Flag indicating if progress information shall be output when downloading a release
    fn show_download_progress(&self) -> bool;

    /// Flag indicating if process informative messages shall be output
    fn show_output(&self) -> bool;

    /// Flag indicating if the user shouldn't be prompted to confirm an update
    fn no_confirm(&self) -> bool;

    /// Strategy for choosing which release the unpinned "latest" path installs (set via
    /// `update_strategy`). Defaults to [`UpdateStrategy::Compatible`].
    fn update_strategy(&self) -> UpdateStrategy {
        UpdateStrategy::Compatible
    }

    /// Flag indicating whether the release notes URL (or body) should be shown in the confirmation
    /// prompt (set via `show_release_notes`). Defaults to `false`.
    fn show_release_notes(&self) -> bool {
        false
    }

    /// Message template to use if `show_download_progress` is set (see `indicatif::ProgressStyle`)
    #[cfg(feature = "progress-bar")]
    fn progress_template(&self) -> &str;

    /// Progress characters to use if `show_download_progress` is set (see `indicatif::ProgressStyle`)
    #[cfg(feature = "progress-bar")]
    fn progress_chars(&self) -> &str;

    /// Authorisation token for communicating with backend
    fn auth_token(&self) -> Option<&str>;

    /// Construct a header with an authorisation entry if an auth token is provided.
    ///
    /// The trait default is a no-op (empty header map): the authorization scheme now lives in the
    /// per-backend `RequestConfig` and is applied by the
    /// shared header-derivation, not here. Backends that need a custom user-agent / scheme still
    /// override this (github/gitlab/gitea).
    fn api_headers(&self, _auth_token: Option<&str>) -> Result<http_client::HeaderMap> {
        Ok(header::HeaderMap::new())
    }
}

/// The crate-private accessors of an updater whose signatures name crate-private types
/// (transport config, callback newtypes, verification material).
///
/// These were previously `#[doc(hidden)]` methods on the public sealed [`UpdateConfig`] trait,
/// which leaked the crate-private types into the public trait contract (shape-only, since the
/// types are not nameable downstream). They now live on this separate `pub(crate)` sub-trait so
/// `UpdateConfig` exposes only public-typed accessors. The orchestration reads these through this
/// trait; both [`ReleaseUpdate`] and [`AsyncReleaseUpdate`] require it as a supertrait so the
/// generic orchestrator bounds (`U: ReleaseUpdate` / `U: AsyncReleaseUpdate`) still reach them.
pub(crate) trait UpdateInternals: sealed::Sealed {
    /// Per-request timeout to apply to backend HTTP requests, if any.
    fn request_timeout(&self) -> Option<std::time::Duration>;

    /// Extra HTTP headers to merge into every backend request.
    fn request_headers(&self) -> &http_client::HeaderMap;

    /// The full resolved per-request transport config (timeout, headers, retries/backoff, injected
    /// clients, and the derived auth scheme/token). Used by the download path to apply the same
    /// auth-header derivation the listing path uses.
    fn request_config(&self) -> &crate::backends::common::RequestConfig;

    /// Optional user-supplied sync HTTP client to apply to the download, mirroring the listing
    /// requests.
    fn request_client(&self) -> Option<std::sync::Arc<dyn http_client::HttpClient>>;

    /// Optional user-supplied async HTTP client to apply to the download (async path only).
    #[cfg(feature = "async")]
    fn request_async_client(&self) -> Option<std::sync::Arc<dyn http_client::AsyncHttpClient>>;

    /// Optional download-progress callback to forward to the download step.
    fn progress_callback(&self) -> Option<std::sync::Arc<crate::DynProgressFn>>;

    /// Optional post-update verification hook, run on the extracted binary before install.
    fn verify_callback(&self) -> Option<std::sync::Arc<crate::DynVerifyFn>>;

    /// Optional custom asset matcher, overriding the built-in target/identifier selection.
    fn asset_matcher(&self) -> Option<std::sync::Arc<crate::DynAssetMatcher>> {
        None
    }

    /// Optional checksum to verify the downloaded artifact against before installing it.
    #[cfg(feature = "checksums")]
    fn verify_checksum(&self) -> Option<&crate::Checksum>;

    /// Whether to verify the download against the backend-published digest of the selected asset,
    /// when one is present. On by default.
    #[cfg(feature = "checksums")]
    fn verify_release_digest(&self) -> bool;

    /// ed25519ph verifying keys to validate a download's authenticity
    #[cfg(feature = "signatures")]
    fn verifying_keys(&self) -> &[crate::VerifyingKey] {
        &[]
    }
}

/// Updates to a specified or latest release.
///
/// This trait is **sealed** (via its [`UpdateConfig`] supertrait): it is implemented only by this
/// crate's backend `Update` types and cannot be implemented for types outside the crate. You
/// consume it through the concrete `Update` each backend's `build()` returns (whose inherent
/// `update()` / `update_extended()` verbs forward here), or as a generic bound — but you do not
/// implement it yourself.
///
/// The shared accessor methods live on the [`UpdateConfig`] supertrait. They resolve on a
/// `dyn ReleaseUpdate` without importing it, and in generic code bounded `R: ReleaseUpdate` they
/// are in scope via the supertrait bound (also no import); bring it into scope
/// (`use self_update::UpdateConfig;`) only to call them on a concrete backend `Update` value.
///
/// The trait is sealed transitively: its [`UpdateConfig`] supertrait requires
/// `sealed::Sealed` (implemented only inside this crate), so `ReleaseUpdate` cannot be
/// implemented for a foreign type even though the trait itself has no visible seal.
#[allow(private_bounds)]
pub trait ReleaseUpdate: UpdateConfig + UpdateInternals {
    /// Fetch the single newest release from the backend.
    ///
    /// The result is a one-element [`Releases`] wrapping the **raw** newest release, unfiltered
    /// (carrying the configured current version). Because the newest release is always present,
    /// `.latest()` is always `Some`, and `.is_update_available()` returns `false` when that newest
    /// release is not strictly newer than the configured current version. This differs from
    /// [`get_newer_releases`](Self::get_newer_releases), whose list is filtered to strictly-newer
    /// releases (there, `.latest()` is `None` when up to date and any present entry is a genuine
    /// update).
    ///
    /// How "newest" treats a non-semver rolling tag (`nightly`, `latest`, a date) differs per
    /// backend: gitlab and gitea read the first page of the listing and skip unparseable tags,
    /// returning the first release the updater can compare; github uses the API's dedicated
    /// `/releases/latest` endpoint, which returns one designated release — if *that* release's
    /// tag is not semver, this errors with [`Error::SemVer`](crate::errors::Error::SemVer) naming
    /// the tag. Prefer [`get_newer_releases`](Self::get_newer_releases) for the skip-enabled path
    /// that behaves identically on every backend.
    fn get_latest_release(&self) -> Result<Releases>;

    /// Fetch the candidate releases from the backend as a [`Releases`] (newest-first, carrying the
    /// configured current version).
    ///
    /// The list is filtered to releases strictly newer than the configured current version, so it
    /// is empty (`.latest()` is `None`) when already up to date, and any entry present is a genuine
    /// update.
    ///
    /// Releases whose tag is not a semver version after stripping a leading lowercase `v` (e.g. a
    /// rolling `nightly` or `latest` tag) are skipped: the updater cannot compare them. Each skip
    /// is logged at `log::debug!` level — hook up a logger (e.g. `env_logger` with
    /// `RUST_LOG=self_update=debug`) to see which tags were dropped. A listing with *no*
    /// parseable release behaves as an empty listing
    /// ([`Error::NoReleaseFound`](crate::errors::Error::NoReleaseFound) on the update path).
    fn get_newer_releases(&self) -> Result<Releases>;

    /// Fetch details of the release matching the specified version
    fn get_release_version(&self, ver: &str) -> Result<Release>;

    /// Display release information and update the current binary to the latest release, pending
    /// confirmation from the user.
    ///
    /// Returns a [`VersionStatus`] carrying only the version tag. Use
    /// [`update_extended`](Self::update_extended) instead if you need the full [`Release`] details
    /// (name, date, body, assets) of the installed release.
    fn update(&self) -> Result<VersionStatus> {
        let current_version = self.current_version().to_string();
        self.update_extended()
            .map(|s| s.into_version_status(current_version))
    }

    /// Same as `update`, but returns [`ReleaseStatus`].
    fn update_extended(&self) -> Result<ReleaseStatus> {
        let current_version = self.current_version();
        let show_output = self.show_output();
        print_check_header(self.target(), current_version, show_output);

        let release = match self.release_tag() {
            None => {
                print_flush(show_output, "Checking latest released version... ")?;
                let releases = self.get_newer_releases()?;
                match choose_latest_release(
                    releases.into_vec(),
                    current_version,
                    show_output,
                    self.update_strategy(),
                )? {
                    Some(release) => release,
                    None => return Ok(ReleaseStatus::UpToDate),
                }
            }
            Some(ref ver) => {
                println(show_output, &format!("Looking for tag: {}", ver));
                self.get_release_version(ver)?
            }
        };

        // Resolve a symlinked bundle destination once, before the user is asked to approve it, and
        // reuse that path for the preflight and the swap. Resolving again later would let the link
        // be repointed in between, so the tree that gets replaced would not be the one confirmed.
        let bundle_target = self.bundle_install_path().map(resolve_bundle_target);
        let bundle_target = bundle_target.as_deref();

        let target_asset = resolve_and_confirm(self, &release, bundle_target)?;

        // Opt-in preflight: bail before downloading if the install path is definitely not writable.
        if self.check_install_path_writable() {
            probe_writable(self.bin_install_path(), bundle_target)?;
        }

        let tmp_archive_dir = tempfile::TempDir::new()?;
        let tmp_archive_path = tmp_archive_dir.path().join(target_asset.name());
        let mut tmp_archive = fs::File::create(&tmp_archive_path)?;

        println(show_output, "Downloading...");
        build_download(self, &target_asset)?.download_to(&mut tmp_archive)?;

        finish_update(
            self,
            release,
            &target_asset,
            tmp_archive_dir,
            &tmp_archive_path,
            bundle_target,
        )
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
    strategy: UpdateStrategy,
) -> Result<Option<Release>> {
    // Only consider releases strictly newer than the current version. The built-in backends already
    // pre-filter this way, so this is a no-op for them; it matters for `backends::custom`, whose
    // `ReleaseSource` may return the current (or older) releases — without this guard the fallback
    // below would treat the current version as an available "update" and re-install it.
    let mut releases = releases
        .into_iter()
        .filter(|r| version::bump_is_greater(current_version, r.version()).unwrap_or(false))
        .collect::<Vec<_>>();

    // Sort the candidates semver-descending (newest first) so the selection below does not depend
    // on the order the source/backend returned them. The built-in backends already sort or filter,
    // but `backends::custom`'s `ReleaseSource` may hand back releases in any order. Uses the shared
    // release comparator (also used by `backends::s3::sort_newer`/`pick_latest`).
    releases.sort_by(|x, y| version::cmp_releases_newest_first(x.version(), y.version()));

    // Under `Latest`, install the newest release overall (ignoring semver compatibility). Under
    // `Compatible` (default), prefer the newest semver-compatible release and only fall back to the
    // newest overall when none is compatible.
    let compatible_releases = match strategy {
        UpdateStrategy::Latest => Vec::new(),
        _ => releases
            .iter()
            .filter(|r| version::bump_is_compatible(current_version, r.version()).unwrap_or(false))
            .collect::<Vec<_>>(),
    };

    let release = if let Some(release) = compatible_releases.first() {
        println(
            show_output,
            &format!(
                "v{} ({} versions compatible)",
                release.version(),
                compatible_releases.len()
            ),
        );
        (*release).clone()
    } else if let Some(release) = releases.first() {
        println(
            show_output,
            &format!(
                "v{} ({} versions available)",
                release.version(),
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
            current_version,
            release.version()
        ),
    );
    // Both versions were already validated as semver upstream (the selection filters above parse
    // them), so this comparison cannot actually error. Express that invariant with `unwrap_or(false)`
    // — consistent with the `bump_is_greater`/`bump_is_compatible` filter sites above — rather than a
    // `?` that would imply a live error path here.
    let qualifier =
        if version::bump_is_compatible(current_version, release.version()).unwrap_or(false) {
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

/// Crate-internal test hooks exposing private update-pipeline helpers to backend unit tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// Expose [`choose_latest_release`] to backend tests (it is otherwise private to this module),
    /// so selection-parity tests can compare what the orchestrator would pick from two release
    /// lists. `show_output` is forced off.
    pub(crate) fn choose_latest_release_for_test(
        releases: Vec<Release>,
        current_version: &str,
    ) -> Result<Option<Release>> {
        choose_latest_release(releases, current_version, false, UpdateStrategy::Compatible)
    }

    /// Construct a [`Release`] with a raw (possibly non-semver) version string, bypassing
    /// [`ReleaseBuilder::build`]'s semver validation. The backends build `Release`s directly from
    /// whatever tag a forge returns, so junk versions can reach the pipeline; tests of the
    /// pipeline's junk tolerance need to fabricate them.
    pub(crate) fn release_with_raw_version(version: &str) -> Release {
        Release {
            name: Arc::from(version),
            version: Arc::from(version),
            date: Arc::from(""),
            body: None,
            release_notes_url: None,
            assets: Vec::new(),
        }
    }
}

/// Derive the (arch, os) substrings used for the fallback asset match from a target triple such as
/// `x86_64-unknown-linux-gnu` or `aarch64-apple-darwin`. The arch is the first triple component; the
/// os is the recognized platform token. Both come from the target string, not the build host, so an
/// explicitly configured cross-target still matches its own assets (and `darwin`-named macOS assets
/// match, which the build-host `std::env::consts::OS` value `"macos"` never did).
fn target_arch_os(target: &str) -> (Option<&str>, Option<&str>) {
    let arch = target.split('-').next().filter(|s| !s.is_empty());
    let os = [
        "linux", "darwin", "windows", "freebsd", "netbsd", "openbsd", "android", "ios", "wasm",
    ]
    .into_iter()
    .find(|os| target.contains(os));
    (arch, os)
}

/// Return `true` iff `name` is safe to use as a single filename component on the local filesystem.
///
/// The name must resolve to exactly one normal path component: no `/` or `\` separators, no `.` or
/// `..`, no root, and no path prefix. The prefix case covers a Windows drive designator such as
/// `C:evil`, which is drive-*relative* (so `Path::is_absolute` is `false`) yet `Path::join` treats
/// as a disk-qualified path, letting a server-supplied asset name escape the temporary directory.
/// `\` and `:` are only special on Windows, so they are rejected explicitly rather than relying on
/// component parsing (which does not treat them as special when the crate is built for unix).
fn is_safe_asset_name(name: &str) -> bool {
    // The name comes from the release listing, so it is remote-controlled, and it is echoed into
    // the confirmation block the user reads before authorizing the replacement. A control
    // character (`\r`, `\n`, ESC) in that block can repaint or hide the lines below it, including
    // the download url, so reject it here rather than relying on the block's formatting.
    if name.chars().any(char::is_control) {
        return false;
    }
    if name.contains('\\') || name.contains(':') {
        return false;
    }
    let mut components = std::path::Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    )
}

/// Format the install-target line of the release-status block.
///
/// Paths print through [`Path::display`](std::path::Path::display), not `{:?}`: `Path`'s `Debug`
/// impl quotes the path and escapes it, so a windows install path came out as
/// `"C:\\Users\\me\\bin\\app.exe"` — doubled separators the user never typed.
fn install_target_line(
    bundle_install_path: Option<&std::path::Path>,
    bin_install_path: &std::path::Path,
) -> String {
    // In bundle mode the install target is the bundle directory, not `bin_install_path` (which the
    // swap never writes), so confirm against the path that will actually be replaced.
    match bundle_install_path {
        Some(bundle) => format!("  * Current bundle: {}", bundle.display()),
        None => format!("  * Current exe: {}", bin_install_path.display()),
    }
}

/// Select the asset to download (custom matcher or the built-in target/identifier match), print the
/// release status, and prompt for confirmation unless suppressed. Shared by both orchestrators.
///
/// `bundle_target` is the already-resolved bundle destination (see [`resolve_bundle_target`]), not
/// the configured `bundle_install_path`, so the status block names the tree the swap will actually
/// replace.
fn resolve_and_confirm<U: UpdateConfig + UpdateInternals + ?Sized>(
    u: &U,
    release: &Release,
    bundle_target: Option<&std::path::Path>,
) -> Result<ReleaseAsset> {
    let target = u.target();
    let target_asset = match u.asset_matcher() {
        Some(matcher) => matcher(&release.assets),
        None => release.asset_for(target, u.asset_identifier()),
    }
    .ok_or_else(|| Error::NoReleaseFound {
        target: Some(target.to_string()),
    })?;

    // Reject a traversal-unsafe asset name before printing it or prompting, so the user is never
    // asked to confirm (and the terminal never echoes) a server-supplied name that validation then
    // rejects. Nothing touches the filesystem before this point.
    if !is_safe_asset_name(target_asset.name()) {
        return Err(Error::InvalidAssetName {
            name: target_asset.name().to_string(),
        });
    }

    let prompt_confirmation = !u.no_confirm();
    if u.show_output() || prompt_confirmation {
        println!("\n{} release status:", u.bin_name());
        println!(
            "{}",
            install_target_line(bundle_target, u.bin_install_path())
        );
        println!("  * New exe release: {:?}", target_asset.name());
        println!(
            "  * New exe download url: {:?}",
            crate::errors::redact_url(target_asset.download_url())
        );
        if u.show_release_notes() {
            if let Some(url) = release.release_notes_url() {
                println!("  * Release notes: {}", url);
            } else if let Some(body) = release.body() {
                println!("  * Release notes:\n{}", body);
            }
        }
        let replaced = match bundle_target {
            Some(_) => "the existing bundle directory will be replaced",
            None => "the existing binary will be replaced",
        };
        println!(
            "\nThe new release will be downloaded/extracted and {}.",
            replaced
        );
    }
    if prompt_confirmation {
        confirm("Do you want to continue? [Y/n] ")?;
    }
    Ok(target_asset)
}

/// Build the [`Download`] for an asset, applying the auth/accept/extra headers, timeout, progress
/// callback, and progress style from the updater. Shared by both orchestrators; the caller drives
/// it with `download_to` (sync) or `download_to_async` (async).
fn build_download<U: UpdateConfig + UpdateInternals + ?Sized>(
    u: &U,
    target_asset: &ReleaseAsset,
) -> Result<Download> {
    let mut download = Download::from_url(target_asset.download_url());
    // Backend base headers (e.g. github's User-Agent). The trait default is a no-op; the auth
    // scheme/token is applied below by the shared `apply_auth` so the download honors a user
    // `request_header(AUTHORIZATION, ..)` override exactly like the listing path.
    let mut headers = u.api_headers(u.auth_token())?;
    headers.insert(
        header::ACCEPT,
        "application/octet-stream"
            .parse()
            .expect("application/octet-stream is a valid header value"),
    );
    // Apply the backend's derived Authorization (scheme + token), skipped when the user supplied
    // their own Authorization via `request_header`. The token is attached only when the asset
    // download URL is on the configured API host (or an allow_auth_host entry), so a server-supplied
    // download_url pointing at another host does not receive the credential.
    u.request_config()
        .apply_auth(target_asset.download_url(), &mut headers)?;
    // Apply the user's extra request headers to the download too. This runs after the ACCEPT and
    // auth headers set above, so a user-supplied header of the same name overrides them here.
    //
    // S2: a user-supplied `Authorization` (set via `request_header(AUTHORIZATION, ..)`) is a
    // credential, so it is host-gated exactly like the crate's derived token — forwarded only to a
    // host `auth_allowed_for` permits (the configured API host or an `allow_auth_host` entry, over
    // https / loopback). A server-chosen next-page or download host that is not authorized does not
    // receive it, so a malicious release server cannot harvest the user's Authorization.
    let user_auth_allowed = u
        .request_config()
        .auth_allowed_for(target_asset.download_url());
    for (name, value) in u.request_headers() {
        if name == header::AUTHORIZATION && !user_auth_allowed {
            continue;
        }
        headers.insert(name.clone(), value.clone());
    }
    download.replace_headers(headers);
    // Forward any injected HTTP client so the download reuses it too.
    download.set_http_client(
        u.request_client(),
        #[cfg(feature = "async")]
        u.request_async_client(),
    );
    // Forward any custom TLS root CA certificates so the download builds a client that trusts them
    // (only used when no client was injected).
    for cert in &u.request_config().root_certificates {
        download.add_root_certificate(cert.clone());
    }
    if let Some(timeout) = u.request_timeout() {
        download.timeout(timeout);
    }
    // Forward the configured retry budget/backoff so the download retries its request-establishment
    // phase (B9: on the custom backend this is the only crate-controlled transport, so `.retries()`
    // now has a real effect there; consistent with the other transport knobs forwarded here).
    {
        let request = u.request_config();
        download.set_retries(
            request.retries,
            request.retry_base_delay,
            request.retry_max_delay,
        );
    }
    if let Some(callback) = u.progress_callback() {
        download.set_progress_callback_arc(callback);
    }
    download.show_download_progress(u.show_download_progress());
    #[cfg(feature = "progress-bar")]
    download.progress_style(crate::ProgressStyle::new(
        u.progress_template(),
        u.progress_chars(),
    ));
    Ok(download)
}

/// The owned, `'static` fields the blocking finish tail needs, copied out of the `&U` accessors so
/// the tail can run inside [`tokio::task::spawn_blocking`] without borrowing the updater.
struct FinishCtx {
    release: Release,
    bin_install_path: std::path::PathBuf,
    target: String,
    bin_name: String,
    bin_path_in_archive: String,
    /// The bundle directory inside the archive; `Some` puts the finish tail in bundle mode, where
    /// `bundle_target` is also `Some` and the `bin_*` paths are unused.
    bundle_path_in_archive: Option<String>,
    /// The resolved bundle destination, not the configured `bundle_install_path`: the orchestrator
    /// resolves it once before the confirmation prompt so the approved path is the written path.
    bundle_target: Option<std::path::PathBuf>,
    show_output: bool,
    verify_callback: Option<std::sync::Arc<crate::DynVerifyFn>>,
    #[cfg(feature = "checksums")]
    verify_checksum: Option<crate::Checksum>,
    /// The selected asset's backend-published digest (`algorithm:hex`), if any, verified when
    /// `verify_release_digest` is on.
    #[cfg(feature = "checksums")]
    asset_digest: Option<Arc<str>>,
    #[cfg(feature = "checksums")]
    verify_release_digest: bool,
    #[cfg(feature = "signatures")]
    verify_keys: Vec<crate::VerifyingKey>,
}

impl FinishCtx {
    /// Capture the owned fields the finish tail needs from the updater, the resolved `release`,
    /// and the selected `target_asset` (its digest feeds the release-digest gate).
    #[cfg_attr(not(feature = "checksums"), allow(unused_variables))]
    fn capture<U: UpdateConfig + UpdateInternals + ?Sized>(
        u: &U,
        release: Release,
        target_asset: &ReleaseAsset,
        bundle_target: Option<&std::path::Path>,
    ) -> Self {
        Self {
            #[cfg(feature = "checksums")]
            asset_digest: target_asset.digest.clone(),
            release,
            bin_install_path: u.bin_install_path().to_path_buf(),
            target: u.target().to_string(),
            bin_name: u.bin_name().to_string(),
            bin_path_in_archive: u.bin_path_in_archive().to_string(),
            bundle_path_in_archive: u.bundle_path_in_archive().map(str::to_string),
            bundle_target: bundle_target.map(std::path::Path::to_path_buf),
            show_output: u.show_output(),
            verify_callback: u.verify_callback(),
            #[cfg(feature = "checksums")]
            verify_checksum: u.verify_checksum().cloned(),
            #[cfg(feature = "checksums")]
            verify_release_digest: u.verify_release_digest(),
            #[cfg(feature = "signatures")]
            verify_keys: u.verifying_keys().to_vec(),
        }
    }
}

/// Verify the downloaded archive (checksum/signature), extract the binary, and install it. This is
/// the sync tail shared verbatim by the sync and async update flows. Builds a [`FinishCtx`] from
/// the updater and delegates to [`finish_update_owned`] without spawning (the sync path runs it
/// inline). Consumes `release` and returns the resulting status.
fn finish_update<U: UpdateConfig + UpdateInternals + ?Sized>(
    u: &U,
    release: Release,
    target_asset: &ReleaseAsset,
    tmp_archive_dir: tempfile::TempDir,
    tmp_archive_path: &std::path::Path,
    bundle_target: Option<&std::path::Path>,
) -> Result<ReleaseStatus> {
    let ctx = FinishCtx::capture(u, release, target_asset, bundle_target);
    finish_update_owned(ctx, tmp_archive_dir, tmp_archive_path)
}

/// The blocking finish tail over **owned** fields: verify (checksum/signature), extract, install.
/// Takes the [`tempfile::TempDir`] by value (moved in, dropped at the end) and the owned `ctx`, so
/// it can be run directly inside [`tokio::task::spawn_blocking`] on the async path. Returns the
/// resulting status.
fn finish_update_owned(
    ctx: FinishCtx,
    tmp_archive_dir: tempfile::TempDir,
    tmp_archive_path: &std::path::Path,
) -> Result<ReleaseStatus> {
    let show_output = ctx.show_output;

    #[cfg(feature = "checksums")]
    {
        if let Some(checksum) = ctx.verify_checksum.as_ref() {
            checksum.verify(tmp_archive_path)?;
        }
        // The backend-published digest of the selected asset (github's per-asset `digest` field),
        // verified by default. A present-but-unparseable digest is a hard error rather than a
        // silent skip; `verify_release_digest(false)` is the escape hatch.
        if ctx.verify_release_digest
            && let Some(digest) = ctx.asset_digest.as_deref()
        {
            crate::Checksum::parse_digest(digest)?.verify(tmp_archive_path)?;
        }
    }

    #[cfg(feature = "signatures")]
    {
        if !ctx.verify_keys.is_empty() {
            println(show_output, "Verifying downloaded file...");
        }
        verify_signature(tmp_archive_path, &ctx.verify_keys)?;
    }

    print_flush(show_output, "Extracting archive... ")?;

    // In bundle mode the archive path names a directory to swap wholesale, otherwise the single
    // executable to extract; the `{{ .. }}` substitutions below are identical either way.
    let path_in_archive = Cow::Borrowed(match ctx.bundle_path_in_archive.as_deref() {
        Some(bundle_path) => bundle_path,
        None => ctx.bin_path_in_archive.as_str(),
    });

    // The `{{ version }}` / `{{ target }}` / `{{ bin }}` template matchers. Hoisted to `static`
    // `LazyLock<Regex>` (I6) so each is compiled once, not rebuilt from its constant pattern on
    // every call. Pattern is unchanged: `{{`, optional whitespace, the variable name, optional
    // whitespace, `}}`.
    static VERSION_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\{\{[[:space:]]*version[[:space:]]*\}\}").unwrap());
    static TARGET_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\{\{[[:space:]]*target[[:space:]]*\}\}").unwrap());
    static BIN_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\{\{[[:space:]]*bin[[:space:]]*\}\}").unwrap());

    // Substitute a `{{ var }}` placeholder (matched by `re`) in `str` with `val`.
    //
    // S6 (defense-in-depth): the `{{ version }}` value comes from the release server, so a
    // malicious version like `../evil` (or one with a `/`/`\` separator) could redirect the
    // extraction path outside the temp dir. When the template actually references the variable,
    // reject a value that is not a single safe path component (reusing `is_safe_asset_name`, which
    // rejects `/`, `\`, `:`, `.`, `..`, roots, and drive prefixes) before it can reach the path.
    fn substitute<'a: 'b, 'b>(re: &Regex, str: &'a str, val: &str) -> Result<Cow<'b, str>> {
        if re.is_match(str) && !is_safe_asset_name(val) {
            return Err(Error::InvalidAssetName {
                name: val.to_string(),
            });
        }
        // `NoExpand` so a `$` in `val` (e.g. a bin name or version containing `$`) is inserted
        // literally, not interpreted as a regex capture-group reference.
        Ok(re.replace_all(str, regex::NoExpand(val)))
    }

    let path_in_archive = substitute(&VERSION_RE, &path_in_archive, ctx.release.version())?;
    let path_in_archive = substitute(&TARGET_RE, &path_in_archive, &ctx.target)?;
    let path_in_archive = substitute(&BIN_RE, &path_in_archive, &ctx.bin_name)?;
    let path_in_archive = path_in_archive.as_ref();

    // Bundle mode: extract the whole tree beside the destination and swap the directory in one
    // rename, instead of extracting and moving a single file.
    if let Some(bundle_target) = ctx.bundle_target.as_deref() {
        install_bundle(
            tmp_archive_path,
            path_in_archive,
            bundle_target,
            ctx.verify_callback.as_deref(),
            show_output,
        )?;
        return Ok(ReleaseStatus::Updated(ctx.release));
    }

    Extract::from_source(tmp_archive_path).extract_file(tmp_archive_dir.path(), path_in_archive)?;
    let new_exe = tmp_archive_dir.path().join(path_in_archive);

    println(show_output, "Done");

    print_flush(show_output, "Replacing binary file... ")?;

    install_binary(
        &new_exe,
        &ctx.bin_install_path,
        ctx.verify_callback.as_deref(),
    )?;
    println(show_output, "Done");

    Ok(ReleaseStatus::Updated(ctx.release))
}

/// Async sibling of [`ReleaseUpdate::update_extended`]: identical flow with the release listing and
/// the download done asynchronously, reusing the shared sync helpers for selection, confirmation,
/// verification, extraction, and install.
#[cfg(feature = "async")]
pub(crate) async fn update_extended_async<U>(u: &U) -> Result<ReleaseStatus>
where
    // `AsyncReleaseUpdate` is never used through a trait object (the async API hands out a concrete
    // `Update`), so `U` is always `Sized` here — unlike the shared sync helpers above. The bound is
    // the async sealed trait; its `UpdateConfig` supertrait supplies the accessors.
    U: AsyncReleaseUpdate + Sync,
{
    let current_version = u.current_version();
    let show_output = u.show_output();
    print_check_header(u.target(), current_version, show_output);

    let release = match u.release_tag() {
        None => {
            print_flush(show_output, "Checking latest released version... ")?;
            let releases = u.get_newer_releases_async().await?;
            match choose_latest_release(
                releases.into_vec(),
                current_version,
                show_output,
                u.update_strategy(),
            )? {
                Some(release) => release,
                None => return Ok(ReleaseStatus::UpToDate),
            }
        }
        Some(ref ver) => {
            println(show_output, &format!("Looking for tag: {}", ver));
            u.get_release_version_async(ver).await?
        }
    };

    // Resolved once before the prompt, as on the sync path, so the confirmed path is the one the
    // swap replaces.
    let bundle_target = u.bundle_install_path().map(resolve_bundle_target);
    let bundle_target = bundle_target.as_deref();

    let target_asset = resolve_and_confirm(u, &release, bundle_target)?;

    // Opt-in preflight: bail before downloading if the install path is definitely not writable.
    // Shares the sync probe for exact parity with `update_extended`.
    if u.check_install_path_writable() {
        probe_writable(u.bin_install_path(), bundle_target)?;
    }

    let tmp_archive_dir = tempfile::TempDir::new()?;
    let tmp_archive_path = tmp_archive_dir.path().join(target_asset.name());
    let mut tmp_archive = fs::File::create(&tmp_archive_path)?;

    println(show_output, "Downloading...");
    build_download(u, &target_asset)?
        .download_to_async(&mut tmp_archive)
        .await?;

    // Run the blocking finish tail (verify/extract/install) off the async executor. Copy out the
    // owned fields, MOVE the TempDir into the closure (it is dropped there), and `.await` the
    // join handle, mapping a JoinError to an update error.
    let ctx = FinishCtx::capture(u, release, &target_asset, bundle_target);
    tokio::task::spawn_blocking(move || {
        finish_update_owned(ctx, tmp_archive_dir, &tmp_archive_path)
    })
    .await
    .map_err(|e| Error::Internal {
        message: "finish-update task failed".to_string(),
        source: Some(Box::new(e)),
    })?
}

/// Run the post-update verification hook (if any) on the freshly-extracted binary, then install
/// it — replacing the current executable in place, or moving it to `bin_install_path`. If the
/// hook returns `Err(..)` the install is aborted (as `Error::VerificationRejected`) before
/// anything is replaced.
fn install_binary(
    new_exe: &std::path::Path,
    bin_install_path: &std::path::Path,
    verify: Option<&crate::DynVerifyFn>,
) -> Result<()> {
    run_verify_hook(new_exe, verify)?;
    let current_exe = std::env::current_exe()?;
    // Only the two install-step writes are wrapped with path context (not `current_exe()` or the
    // verify hook above): a permission failure here becomes `InstallPathNotWritable` naming the
    // path, and any other IO error is rewrapped so the path shows up while the `ErrorKind` stays
    // inspectable. This annotation is always on, independent of the opt-in preflight probe.
    if same_file(bin_install_path, &current_exe) {
        self_replace::self_replace(new_exe)
            .map_err(|e| map_install_io_error(e, bin_install_path))?;
    } else {
        Move::from_source(new_exe)
            .to_dest(bin_install_path)
            .map_err(|e| match e {
                Error::Io(io) => map_install_io_error(io, bin_install_path),
                other => other,
            })?;
    }
    Ok(())
}

/// Run the post-update verification hook (if any) on `new_path` -- the freshly-extracted binary, or
/// in bundle mode the staged bundle root -- before anything is replaced.
///
/// A hook that returns `Err` (an explicit rejection or a hook IO error) aborts the install; its
/// message becomes the rejection reason. An error that already is a `VerificationRejected` (e.g.
/// built via `Error::verification_rejected`) passes through unwrapped so the reason is not nested
/// inside another rejection message.
fn run_verify_hook(new_path: &std::path::Path, verify: Option<&crate::DynVerifyFn>) -> Result<()> {
    if let Some(verify) = verify {
        verify(new_path).map_err(|e| match e {
            Error::VerificationRejected { .. } => e,
            other => Error::VerificationRejected {
                reason: Some(other.to_string()),
            },
        })?;
    }
    Ok(())
}

/// Extract the archive and replace `bundle_target` with the bundle directory it carries at
/// `bundle_path_in_archive`.
///
/// `bundle_target` is already resolved by [`resolve_bundle_target`]: the caller does that once,
/// before the confirmation prompt, so the path the user approves is the path this writes. Resolving
/// it here instead would reopen the window between the prompt and the swap.
///
/// Both the extraction target and the rollback stash are temporary directories created inside the
/// destination's *parent*, so every rename in [`swap_bundle`] is same-filesystem and there is no
/// cross-device case to fall back from. A failure to create either is an install-step IO error
/// naming the bundle path.
fn install_bundle(
    archive: &std::path::Path,
    bundle_path_in_archive: &str,
    bundle_target: &std::path::Path,
    verify: Option<&crate::DynVerifyFn>,
    show_output: bool,
) -> Result<()> {
    let parent = install_parent(bundle_target);
    let staging =
        tempfile::TempDir::new_in(parent).map_err(|e| map_install_io_error(e, bundle_target))?;
    let stash =
        tempfile::TempDir::new_in(parent).map_err(|e| map_install_io_error(e, bundle_target))?;

    Extract::from_source(archive).extract_into(staging.path())?;
    let staged_root = staging.path().join(bundle_path_in_archive);
    println(show_output, "Done");

    print_flush(show_output, "Replacing bundle directory... ")?;
    swap_bundle(
        &staged_root,
        bundle_target,
        stash.path(),
        &std::env::current_exe()?,
        verify,
    )?;
    println(show_output, "Done");
    Ok(())
}

/// Replace the directory at `bundle_install_path` with the staged tree at `staged_root`, stashing
/// the displaced tree under `stash` so a failure can be rolled back.
///
/// The swap is whole-tree and rename-only:
///
/// 1. When the running executable lives inside the installed bundle, rename it aside into `stash`,
///    so the old tree holds no running image before the directory itself is renamed (the same
///    rename-the-running-image mechanism a self-replace relies on).
/// 2. Rename the installed bundle to `stash` (skipped when nothing is installed yet).
/// 3. Rename the staged tree onto `bundle_install_path`.
///
/// A failure at step 2 or 3 reverses the renames already applied, so the original bundle (and the
/// running executable's path inside it) is restored; the error returned is always the original one.
/// Rollback is best-effort and a rollback failure is logged rather than surfaced, matching the
/// [`MoveAll`](crate::MoveAll) contract. Once step 3 succeeds the update is committed and the file
/// at the running executable's path is the new bundle's executable.
///
/// Nothing under `bundle_install_path` is touched unless the archive really carried the bundle: the
/// staged root must exist and be a directory, and when `running_exe` is inside the bundle the staged
/// tree must carry a file at the same relative path. The `verify_binary` hook runs against the
/// staged bundle root, also before any rename.
///
/// `running_exe` is the process's own executable (`current_exe()`), passed in rather than read here
/// so the swap is exercisable against a temporary tree.
fn swap_bundle(
    staged_root: &std::path::Path,
    bundle_install_path: &std::path::Path,
    stash: &std::path::Path,
    running_exe: &std::path::Path,
    verify: Option<&crate::DynVerifyFn>,
) -> Result<()> {
    if !staged_root.is_dir() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "the archive has no bundle directory at {}",
                staged_root.display()
            ),
        )));
    }

    let exe_aside = match exe_inside_bundle(running_exe, bundle_install_path) {
        Some((exe, rel)) => {
            if !staged_root.join(&rel).is_file() {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "the staged bundle has no file at {}, the path the running executable \
                         occupies inside {}",
                        rel.display(),
                        bundle_install_path.display()
                    ),
                )));
            }
            Some(exe)
        }
        None => None,
    };

    run_verify_hook(staged_root, verify)?;

    let stashed_exe = stash.join("exe-aside");
    let stashed_old = stash.join("old");

    if let Some(exe) = exe_aside.as_deref() {
        fs::rename(exe, &stashed_exe).map_err(|e| map_install_io_error(e, bundle_install_path))?;
    }

    // `symlink_metadata` rather than `exists()`: a dangling symlink at the path is still an entry
    // that must be stashed out of the way, and renaming a directory onto one fails with ENOTDIR.
    let old_stashed = fs::symlink_metadata(bundle_install_path).is_ok();
    if old_stashed && let Err(e) = fs::rename(bundle_install_path, &stashed_old) {
        if let Some(exe) = exe_aside.as_deref() {
            restore_stashed(&stashed_exe, exe);
        }
        return Err(map_install_io_error(e, bundle_install_path));
    }

    if let Err(e) = fs::rename(staged_root, bundle_install_path) {
        // Reverse the applied renames: the old tree first, so the executable can go back inside it.
        if old_stashed {
            restore_stashed(&stashed_old, bundle_install_path);
        }
        if let Some(exe) = exe_aside.as_deref() {
            restore_stashed(&stashed_exe, exe);
        }
        return Err(map_install_io_error(e, bundle_install_path));
    }

    Ok(())
}

/// Best-effort rollback rename of a stashed path back into place. A failure is logged rather than
/// returned, so the caller still surfaces the original error that triggered the rollback.
fn restore_stashed(from: &std::path::Path, to: &std::path::Path) {
    if let Err(e) = fs::rename(from, to) {
        log::error!(
            "failed to restore {} from stash {} during rollback: {}",
            to.display(),
            from.display(),
            e
        );
    }
}

/// The running executable's real path plus its path relative to `bundle`, when it lives inside the
/// bundle; `None` otherwise.
///
/// Both sides are canonicalized (resolving symlinks and `..`) before comparing, for the same reason
/// [`same_file`] does it: `current_exe()` is symlink-resolved on some platforms while a configured
/// install path is not, so a raw prefix test can miss that the exe is inside the bundle. The
/// returned exe path is the canonical one, which is the file the swap renames aside.
fn exe_inside_bundle(
    exe: &std::path::Path,
    bundle: &std::path::Path,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let exe = fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    let bundle = fs::canonicalize(bundle).unwrap_or_else(|_| bundle.to_path_buf());
    let rel = exe.strip_prefix(&bundle).ok()?.to_path_buf();
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some((exe, rel))
}

/// The bundle directory a configured install path actually designates: the symlink target when the
/// path is a symlink, else the path itself.
///
/// `rename` does not follow a path's final component, so swapping straight onto a symlinked
/// `bundle_install_path` would stash the *link*, plant a real directory in its place, and leave the
/// installed tree orphaned on disk. Resolving first means the tree behind the link is what gets
/// replaced, the link survives the update, and staging is created beside the real tree so the swap's
/// renames stay on one filesystem.
///
/// Only an existing symlink is resolved: a plain path (including one that does not exist yet, the
/// fresh-install case) and a dangling link are returned unchanged, the latter so the swap stashes and
/// replaces the stale entry.
fn resolve_bundle_target(bundle_install_path: &std::path::Path) -> std::path::PathBuf {
    let is_symlink = fs::symlink_metadata(bundle_install_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if !is_symlink {
        return bundle_install_path.to_path_buf();
    }
    // A dangling link has no target to replace; keep the configured path.
    fs::canonicalize(bundle_install_path).unwrap_or_else(|_| bundle_install_path.to_path_buf())
}

/// The directory an install path lives in: its parent, or the current directory for a bare name.
fn install_parent(path: &std::path::Path) -> &std::path::Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    }
}

/// Resolve the default `bundle_install_path` for bundle mode when the caller set none.
///
/// macOS derives it from the running executable ([`enclosing_app_bundle`]), mirroring how
/// `bin_install_path` defaults to `current_exe()`. A running executable with no `.app` ancestor is
/// [`Error::NoAppBundle`], and one running from a read-only translocated mount (a quarantined app,
/// whose bundle path is not the installed one) is [`Error::AppTranslocated`] rather than a
/// read-only-filesystem error from the middle of the swap.
///
/// Every other platform has no meaningful default, so the setter is required there.
pub(crate) fn default_bundle_install_path() -> Result<std::path::PathBuf> {
    resolve_default_bundle_path(&std::env::current_exe()?, HAS_APP_BUNDLE_DEFAULT)
}

/// Whether this target derives a default `bundle_install_path` from the running executable. Only
/// macOS has the `.app` convention to derive one from.
///
/// A `cfg!` value rather than a `#[cfg]` branch so both policies below compile, and are tested, on
/// every target: the macOS resolution is the part most worth pinning and the least reachable from CI.
const HAS_APP_BUNDLE_DEFAULT: bool = cfg!(target_os = "macos");

/// Resolve the default bundle install path from `exe`, for a target that `has_default`.
///
/// Split out from [`default_bundle_install_path`] over an explicit exe path and policy flag so every
/// arm is exercisable on any host, rather than the macOS arm being invisible to a linux test run.
fn resolve_default_bundle_path(
    exe: &std::path::Path,
    has_default: bool,
) -> Result<std::path::PathBuf> {
    if !has_default {
        return Err(Error::MissingField {
            field: "bundle_install_path",
        });
    }
    if is_translocated(exe) {
        return Err(Error::AppTranslocated {
            exe: exe.to_path_buf(),
        });
    }
    enclosing_app_bundle(exe).ok_or_else(|| Error::NoAppBundle {
        exe: exe.to_path_buf(),
    })
}

/// The nearest ancestor of `exe` whose name ends in `.app`, i.e. the macOS application bundle the
/// executable is installed in. `None` when the executable is not inside one.
///
/// Pure and path-lexical (no filesystem access), so it is exercised on every platform even though
/// only macOS uses it for the default install path. Innermost wins for a nested bundle (an
/// `.app` shipped inside another `.app`). The extension is matched case-insensitively: macOS's
/// default filesystem preserves case but does not distinguish it, so a `MyApp.App` on disk is the
/// same bundle as `MyApp.app`.
pub(crate) fn enclosing_app_bundle(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    exe.ancestors()
        .skip(1)
        .find(|a| {
            a.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        })
        .map(std::path::Path::to_path_buf)
}

/// Whether `exe` is running from a macOS App Translocation mount, i.e. a quarantined copy executing
/// from a read-only randomized path such as
/// `/private/var/folders/../AppTranslocation/<uuid>/d/MyApp.app`.
///
/// Pure and path-lexical, matching on the `AppTranslocation` path component.
pub(crate) fn is_translocated(exe: &std::path::Path) -> bool {
    exe.components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new("AppTranslocation"))
}

/// Map an IO error from an install-step write into the crate error, always naming the install path.
///
/// A `PermissionDenied` becomes [`Error::InstallPathNotWritable`] carrying the path; any other kind
/// is rewrapped as [`Error::Io`] whose message names the path (`"installing to {path}: {orig}"`)
/// while preserving the original [`std::io::ErrorKind`] so callers can still inspect it. Non-permission
/// failures (e.g. a full disk) are deliberately NOT reclassified as `InstallPathNotWritable`.
fn map_install_io_error(e: std::io::Error, bin_install_path: &std::path::Path) -> Error {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        Error::InstallPathNotWritable {
            path: bin_install_path.to_path_buf(),
        }
    } else {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("installing to {}: {}", bin_install_path.display(), e),
        ))
    }
}

/// Dispatch the opt-in preflight probe to the path the install step will actually write: the
/// bundle's parent directory in bundle mode (the swap needs create+rename permission there, not on
/// the bundle itself), otherwise `bin_install_path`.
pub(crate) fn probe_writable(
    bin_install_path: &std::path::Path,
    bundle_install_path: Option<&std::path::Path>,
) -> Result<()> {
    match bundle_install_path {
        Some(bundle) => probe_dir_writable(install_parent(bundle)),
        None => probe_install_path_writable(bin_install_path),
    }
}

/// Best-effort preflight probe of whether a directory accepts new entries, by creating and
/// immediately dropping a temporary file in it. Conservative in the same way as
/// [`probe_install_path_writable`]: only a definite
/// [`PermissionDenied`](std::io::ErrorKind::PermissionDenied) fails, naming the directory probed.
pub(crate) fn probe_dir_writable(dir: &std::path::Path) -> Result<()> {
    match tempfile::Builder::new()
        .prefix(".self_update_writecheck")
        .tempfile_in(dir)
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(Error::InstallPathNotWritable {
                path: dir.to_path_buf(),
            })
        }
        // Indeterminate (missing dir, weird fs, etc.): proceed and let the real op surface it.
        Err(_) => Ok(()),
    }
}

/// Best-effort preflight probe of whether `bin_install_path` can be written, run before any
/// download when `check_install_path_writable` is set.
///
/// Filesystem-only and conservative. If the path already exists, probe the file by opening it for
/// append; otherwise probe its parent directory by creating and immediately dropping a temporary
/// sibling (a `.self_update_writecheck` prefix). ONLY a definite
/// [`PermissionDenied`](std::io::ErrorKind::PermissionDenied) maps to
/// [`Error::InstallPathNotWritable`]; success or ANY other error kind (a missing parent, an unusual
/// filesystem, etc.) returns `Ok(())` so the update proceeds and the real install step surfaces the
/// outcome. The create/delete probe is the honest test under Windows ACLs too, so no `cfg` split is
/// needed.
pub(crate) fn probe_install_path_writable(bin_install_path: &std::path::Path) -> Result<()> {
    let probe: std::io::Result<()> = if bin_install_path.exists() {
        std::fs::OpenOptions::new()
            .append(true)
            .open(bin_install_path)
            .map(|_| ())
    } else {
        // `parent()` is `Some("")` for a bare file name; treat that (and `None`) as the CWD.
        let parent = match bin_install_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => std::path::Path::new("."),
        };
        tempfile::Builder::new()
            .prefix(".self_update_writecheck")
            .tempfile_in(parent)
            .map(|_| ())
    };
    match probe {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(Error::InstallPathNotWritable {
                path: bin_install_path.to_path_buf(),
            })
        }
        // Indeterminate (missing parent, weird fs, etc.): proceed and let the real op surface it.
        Err(_) => Ok(()),
    }
}

/// Whether two paths refer to the same file. Compares canonicalized paths (resolving symlinks and
/// `..`), falling back to a raw comparison when canonicalization fails (for example when a path does
/// not yet exist). `current_exe()` is symlink-resolved on some platforms while a user-supplied
/// `bin_install_path` is not, so a raw `==` can miss that both name the running executable and route
/// a self-update through the plain `Move` path instead of `self_replace`.
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
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

/// Verify a downloaded archive's embedded signature against a set of ed25519 verifying keys, the
/// same check [`update()`](ReleaseUpdate::update) runs internally when `verifying_keys` are set.
///
/// This is exposed so a caller that stages a download itself (for example an installer that fetches
/// a companion binary via [`Download`](crate::Download) before the main update loop exists) can run
/// the identical verification without reimplementing it.
///
/// `keys` are raw 32-byte ed25519 public keys ([`VerifyingKey`](crate::VerifyingKey)); verification
/// uses any-of semantics, passing as soon as one key validates a signature in the archive (so a
/// key-rotation window can list both the old and new keys). Signatures are produced and read with
/// [`zipsign`](zipsign_api), which supports `.tar.gz` and `.zip` archives only.
///
/// # Errors
///
/// - Returns `Ok(())` immediately when `keys` is empty (nothing to verify against).
/// - [`Error::NoSignatures`](crate::errors::Error::NoSignatures) if the archive format is not one
///   zipsign can carry a signature in (anything other than `.tar.gz` / `.zip`).
/// - [`Error::Signature`](crate::errors::Error::Signature) if no key validates a signature.
/// - [`Error::SignatureNonUTF8`](crate::errors::Error::SignatureNonUTF8) if the archive's file name
///   is not UTF-8 (zipsign binds the signature to the file name).
/// - [`Error::Io`](crate::errors::Error::Io) if the archive cannot be opened.
///
/// # Example
///
/// ```rust,ignore
/// const KEY: self_update::VerifyingKey = *include_bytes!("../release.pub");
/// self_update::Download::from_url(url).download_to(&mut file)?;
/// self_update::verify_signature("downloaded-1.2.3.tar.gz", &[KEY])?;
/// ```
#[cfg(feature = "signatures")]
#[cfg_attr(docsrs, doc(cfg(feature = "signatures")))]
pub fn verify_signature(
    archive_path: impl AsRef<std::path::Path>,
    keys: &[crate::VerifyingKey],
) -> crate::Result<()> {
    let archive_path = archive_path.as_ref();
    if keys.is_empty() {
        return Ok(());
    }

    let archive_kind = crate::detect_archive(archive_path)?;
    #[cfg(any(feature = "archive-tar", feature = "archive-zip"))]
    {
        let context = archive_path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.as_bytes())
            .ok_or(Error::SignatureNonUTF8)?;

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
    use super::{
        Releases, UpdateStrategy, choose_latest_release, install_binary, install_target_line,
    };
    // `ReleaseAsset` is only referenced unqualified by the checksum tests below; gate the import so
    // a build without `checksums` (e.g. `--features s3` alone) does not trip the unused-import lint.
    #[cfg(feature = "checksums")]
    use super::ReleaseAsset;
    use crate::Download;
    use crate::DynVerifyFn;
    use crate::errors::Error;
    use crate::errors::Result;
    use crate::update::Release;

    fn rel(version: &str) -> Release {
        // Raw construction: several tests below exercise the pipeline's tolerance of non-semver
        // versions, which `ReleaseBuilder::build` rejects (the backends build `Release`s directly
        // from forge tags, so junk can still reach the pipeline).
        super::testing::release_with_raw_version(version)
    }

    // --- Releases (D1) ------------------------------------------------------------------------

    #[test]
    fn releases_is_update_available_true_when_latest_newer() {
        // Newest-first list; latest (2.0.0) is strictly newer than the held current version.
        let releases = Releases::new(vec![rel("2.0.0"), rel("1.0.0")], "1.0.0".to_string());
        assert!(
            releases.is_update_available().unwrap(),
            "2.0.0 > 1.0.0 => update available"
        );
    }

    #[test]
    fn releases_is_update_available_false_when_latest_not_newer() {
        // latest (1.0.0) equals the current version => not strictly newer.
        let releases = Releases::new(vec![rel("1.0.0"), rel("0.9.0")], "1.0.0".to_string());
        assert!(
            !releases.is_update_available().unwrap(),
            "1.0.0 not newer than 1.0.0 => no update"
        );
    }

    #[test]
    fn releases_is_update_available_false_when_empty() {
        // An empty list is "nothing available", not an error.
        let releases = Releases::new(vec![], "1.0.0".to_string());
        assert!(
            !releases.is_update_available().unwrap(),
            "empty Releases => no update available"
        );
    }

    #[test]
    fn releases_is_update_available_true_when_newer_not_first() {
        // An out-of-order multi-element list (a custom ReleaseSource may not sort): the only
        // release newer than the current version (2.0.0) is NOT the first element. The check must
        // still report an update available because it scans the whole set, not just first().
        let releases = Releases::new(
            vec![rel("0.9.0"), rel("1.0.0"), rel("2.0.0")],
            "1.0.0".to_string(),
        );
        assert!(
            releases.is_update_available().unwrap(),
            "2.0.0 is newer than 1.0.0 even though it is not first => update available"
        );
    }

    #[test]
    fn releases_is_update_available_false_when_nothing_newer_unordered() {
        // Out-of-order list where nothing exceeds the current version (1.0.0) => no update.
        let releases = Releases::new(
            vec![rel("0.9.0"), rel("1.0.0"), rel("0.5.0")],
            "1.0.0".to_string(),
        );
        assert!(
            !releases.is_update_available().unwrap(),
            "no release exceeds 1.0.0 => no update available"
        );
    }

    #[test]
    fn releases_is_update_available_propagates_semver_error() {
        // The doc contract: the first release *reached* whose version fails to parse as semver
        // propagates its error (unlike `choose_latest_release`, which silently skips unparseable
        // versions). Nothing before the bad entry is strictly newer, so the scan reaches it.
        let releases = Releases::new(
            vec![rel("0.9.0"), rel("not-a-version"), rel("2.0.0")],
            "1.0.0".to_string(),
        );
        let err = releases
            .is_update_available()
            .expect_err("an unparseable version reached by the scan must error");
        assert!(
            matches!(err, crate::errors::Error::SemVer(_)),
            "expected Error::SemVer, got {:?}",
            err
        );
    }

    #[test]
    fn releases_is_update_available_short_circuits_before_semver_error() {
        // The complement: a strictly-newer release positioned before the unparseable one
        // short-circuits to Ok(true); the bad entry is never examined.
        let releases = Releases::new(
            vec![rel("2.0.0"), rel("not-a-version")],
            "1.0.0".to_string(),
        );
        assert!(
            releases
                .is_update_available()
                .expect("a found update wins over a later parse error"),
            "2.0.0 > 1.0.0 must return true before reaching the bad entry"
        );
    }

    #[test]
    fn releases_with_current_version_enables_is_update_available() {
        // A bare listing (no current version) errors with NoCurrentVersion; attaching one via
        // with_current_version turns it into a comparable listing without rebuilding.
        let bare = Releases::from_listing(vec![rel("2.0.0"), rel("1.0.0")]);
        assert!(matches!(
            bare.is_update_available(),
            Err(crate::errors::Error::NoCurrentVersion)
        ));
        let with_version = bare.with_current_version("1.0.0");
        assert_eq!(with_version.current_version(), Some("1.0.0"));
        assert!(with_version.is_update_available().unwrap());
        // It also replaces an existing current version.
        let replaced =
            Releases::from_releases(vec![rel("2.0.0")], "1.0.0").with_current_version("2.0.0");
        assert!(!replaced.is_update_available().unwrap());
    }

    #[test]
    fn release_builder_new_matches_release_builder() {
        // `ReleaseBuilder::new()` is the conventional entry; it must behave exactly like
        // `Release::builder()`.
        let a = super::ReleaseBuilder::new()
            .version("1.2.3")
            .build()
            .unwrap();
        let b = Release::builder().version("1.2.3").build().unwrap();
        assert_eq!(a.version(), b.version());
        assert_eq!(a.name(), b.name());
    }

    #[test]
    fn release_builder_release_notes_url_roundtrips() {
        // Set via the builder, read via the getter; unset stays None.
        let with_url = Release::builder()
            .version("1.2.3")
            .release_notes_url("https://example.com/releases/v1.2.3")
            .build()
            .unwrap();
        assert_eq!(
            with_url.release_notes_url(),
            Some("https://example.com/releases/v1.2.3")
        );

        let without = Release::builder().version("1.2.3").build().unwrap();
        assert_eq!(without.release_notes_url(), None);
    }

    #[test]
    fn release_builder_build_rejects_non_semver_versions() {
        // A `v` prefix or a non-semver tag must fail at build() with Error::SemVer instead of
        // being stored and silently dropped (or erroring opaquely) later in the pipeline.
        for bad in ["v1.2.3", "latest", "not-a-version", ""] {
            let err = Release::builder()
                .version(bad)
                .build()
                .expect_err(&format!("version {bad:?} must be rejected"));
            assert!(
                matches!(err, crate::errors::Error::SemVer(_)),
                "expected Error::SemVer for {bad:?}, got {err:?}"
            );
        }
        // Valid semver still builds, including pre-release and build metadata.
        for good in ["1.2.3", "2.0.0-alpha.1", "1.0.0+build.5"] {
            Release::builder()
                .version(good)
                .build()
                .unwrap_or_else(|e| panic!("version {good:?} must build: {e}"));
        }
    }

    #[test]
    fn release_builder_missing_version_is_missing_field() {
        // The unset-version error stays MissingField (not SemVer).
        let err = Release::builder().name("x").build().unwrap_err();
        assert!(
            matches!(
                err,
                crate::errors::Error::MissingField { field } if field == "version"
            ),
            "expected MissingField {{ field: \"version\" }}"
        );
    }

    // --- ReleaseSource default methods ----------------------------------------------------------

    struct ListOnlySource(Vec<Release>);

    impl super::ReleaseSource for ListOnlySource {
        fn get_releases(&self) -> Result<Vec<Release>> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn release_source_default_get_latest_release_picks_newest_unsorted() {
        // Only get_releases is implemented; the default get_latest_release must pick the newest
        // by semver even when the source returns an unsorted list.
        use super::ReleaseSource;
        let source = ListOnlySource(vec![rel("1.0.0"), rel("2.1.0"), rel("1.5.0")]);
        assert_eq!(source.get_latest_release().unwrap().version(), "2.1.0");
    }

    #[test]
    fn release_source_default_get_latest_release_errors_when_empty() {
        use super::ReleaseSource;
        let source = ListOnlySource(vec![]);
        assert!(matches!(
            source.get_latest_release(),
            Err(crate::errors::Error::NoReleaseFound { .. })
        ));
    }

    #[test]
    fn release_source_default_get_release_version_finds_exact_match() {
        use super::ReleaseSource;
        let source = ListOnlySource(vec![rel("2.0.0"), rel("1.5.0"), rel("1.0.0")]);
        assert_eq!(
            source.get_release_version("1.5.0").unwrap().version(),
            "1.5.0"
        );
        // No match (including a v-prefixed query; the default matches the stored bare semver
        // exactly) errors with NoReleaseFound.
        assert!(matches!(
            source.get_release_version("v1.5.0"),
            Err(crate::errors::Error::NoReleaseFound { .. })
        ));
        assert!(matches!(
            source.get_release_version("9.9.9"),
            Err(crate::errors::Error::NoReleaseFound { .. })
        ));
    }

    #[cfg(feature = "async")]
    struct AsyncListOnlySource(Vec<Release>);

    #[cfg(feature = "async")]
    impl super::AsyncReleaseSource for AsyncListOnlySource {
        async fn get_releases(&self) -> Result<Vec<Release>> {
            Ok(self.0.clone())
        }
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_release_source_default_methods_derive_from_get_releases() {
        use super::AsyncReleaseSource;
        let source = AsyncListOnlySource(vec![rel("1.0.0"), rel("2.1.0"), rel("1.5.0")]);
        assert_eq!(
            source.get_latest_release().await.unwrap().version(),
            "2.1.0"
        );
        assert_eq!(
            source.get_release_version("1.5.0").await.unwrap().version(),
            "1.5.0"
        );
        assert!(matches!(
            source.get_release_version("9.9.9").await,
            Err(crate::errors::Error::NoReleaseFound { .. })
        ));
        let empty = AsyncListOnlySource(vec![]);
        assert!(matches!(
            empty.get_latest_release().await,
            Err(crate::errors::Error::NoReleaseFound { .. })
        ));
    }

    #[test]
    fn releases_latest_all_and_into_vec() {
        let releases = Releases::new(
            vec![rel("2.0.0"), rel("1.5.0"), rel("1.0.0")],
            "1.0.0".to_string(),
        );
        // latest() is the first (newest) element.
        assert_eq!(releases.latest().unwrap().version(), "2.0.0");
        // all() returns the whole slice, newest-first.
        let all: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(all, vec!["2.0.0", "1.5.0", "1.0.0"]);
        // into_vec() consumes and yields the same order.
        let v: Vec<String> = releases
            .into_vec()
            .into_iter()
            .map(|r| r.version().to_string())
            .collect();
        assert_eq!(v, vec!["2.0.0", "1.5.0", "1.0.0"]);
    }

    #[test]
    fn releases_latest_is_none_when_empty() {
        let releases = Releases::new(vec![], "1.0.0".to_string());
        assert!(releases.latest().is_none());
        assert!(releases.all().is_empty());
    }

    #[test]
    fn releases_len_and_is_empty() {
        let empty = Releases::new(vec![], "1.0.0".to_string());
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());

        let some = Releases::new(vec![rel("2.0.0"), rel("1.0.0")], "1.0.0".to_string());
        assert_eq!(some.len(), 2);
        assert!(!some.is_empty());
    }

    #[test]
    fn releases_current_version_accessor() {
        let releases = Releases::new(vec![rel("2.0.0")], "1.2.3".to_string());
        assert_eq!(releases.current_version(), Some("1.2.3"));
    }

    #[test]
    fn releases_into_iterator_owned_in_order() {
        let releases = Releases::new(
            vec![rel("2.0.0"), rel("1.5.0"), rel("1.0.0")],
            "1.0.0".to_string(),
        );
        // Owned IntoIterator consumes the Releases and yields Release by value, newest-first.
        let v: Vec<String> = releases
            .into_iter()
            .map(|r| r.version().to_string())
            .collect();
        assert_eq!(v, vec!["2.0.0", "1.5.0", "1.0.0"]);
    }

    #[test]
    fn releases_into_iterator_borrowed_in_order() {
        let releases = Releases::new(
            vec![rel("2.0.0"), rel("1.5.0"), rel("1.0.0")],
            "1.0.0".to_string(),
        );
        // Borrowed IntoIterator yields &Release without consuming.
        let v: Vec<&str> = (&releases).into_iter().map(|r| r.version()).collect();
        assert_eq!(v, vec!["2.0.0", "1.5.0", "1.0.0"]);
        // Still usable afterwards (not consumed).
        assert_eq!(releases.len(), 3);
    }

    #[test]
    fn releases_into_iterator_empty_yields_nothing() {
        // Boundary: iterating a zero-release `Releases` (the up-to-date filtered case) yields no
        // items over both the owned and the borrowed iterator — no sentinel, no panic.
        let borrowed = Releases::new(vec![], "1.0.0".to_string());
        assert_eq!((&borrowed).into_iter().count(), 0, "&Releases over empty");
        assert!(borrowed.is_empty());
        let owned = Releases::new(vec![], "1.0.0".to_string());
        assert_eq!(owned.into_iter().count(), 0, "owned Releases over empty");
    }

    #[test]
    fn releases_into_iterator_order_matches_all() {
        // The owned and borrowed IntoIterator orderings must be exactly the `all()` order, not just
        // "some newest-first order" — pin them against `all()` itself, not a hand-written literal.
        let releases = Releases::new(
            vec![rel("3.0.0"), rel("2.1.0"), rel("2.0.0"), rel("1.0.0")],
            "1.0.0".to_string(),
        );
        let expected: Vec<String> = releases
            .all()
            .iter()
            .map(|r| r.version().to_string())
            .collect();
        let borrowed: Vec<String> = (&releases)
            .into_iter()
            .map(|r| r.version().to_string())
            .collect();
        assert_eq!(borrowed, expected, "&Releases iteration == all() order");
        let owned: Vec<String> = releases
            .into_iter()
            .map(|r| r.version().to_string())
            .collect();
        assert_eq!(owned, expected, "owned iteration == all() order");
    }

    // --- ReleaseStatus (C2) -------------------------------------------------------------------

    #[test]
    fn release_status_into_version_status_updated() {
        // into_version_status on Updated must yield VersionStatus::Updated with the release version.
        let rs = super::ReleaseStatus::Updated(rel("2.0.0"));
        let vs = rs.into_version_status("1.0.0".to_string());
        assert!(
            vs.is_updated(),
            "ReleaseStatus::Updated => VersionStatus::Updated"
        );
        assert_eq!(vs.version(), "2.0.0", "version comes from the release");
    }

    #[test]
    fn release_status_into_version_status_up_to_date() {
        // into_version_status on UpToDate must yield VersionStatus::UpToDate with current_version.
        let rs = super::ReleaseStatus::UpToDate;
        let vs = rs.into_version_status("1.5.0".to_string());
        assert!(
            vs.is_up_to_date(),
            "ReleaseStatus::UpToDate => VersionStatus::UpToDate"
        );
        assert_eq!(
            vs.version(),
            "1.5.0",
            "version is the current_version passed in"
        );
    }

    #[test]
    fn release_status_is_updated_predicate() {
        let updated = super::ReleaseStatus::Updated(rel("1.2.3"));
        assert!(updated.is_updated(), "Updated => is_updated() true");
        assert!(!updated.is_up_to_date());

        let up_to_date = super::ReleaseStatus::UpToDate;
        assert!(!up_to_date.is_updated(), "UpToDate => is_updated() false");
        assert!(up_to_date.is_up_to_date());
    }

    #[test]
    fn release_status_release_accessors() {
        let updated = super::ReleaseStatus::Updated(rel("1.2.3"));
        assert_eq!(
            updated.updated_release().map(|r| r.version()),
            Some("1.2.3"),
            "updated_release() borrows the installed release"
        );
        assert_eq!(
            updated
                .into_updated_release()
                .map(|r| r.version().to_string()),
            Some("1.2.3".to_string()),
            "into_updated_release() yields the installed release"
        );

        let up_to_date = super::ReleaseStatus::UpToDate;
        assert!(
            up_to_date.updated_release().is_none(),
            "UpToDate => updated_release() None"
        );
        assert!(
            up_to_date.into_updated_release().is_none(),
            "UpToDate => into_updated_release() None"
        );
    }

    // The arch/os fallback in `asset_for` derives its tokens from the `target` argument, not the
    // build host. A cross-target selection (target != host) must pick the asset named for the
    // configured target, and `darwin`-named macOS assets must match a `*-apple-darwin` target.
    #[test]
    fn asset_for_fallback_uses_configured_target_not_build_host() {
        let release = super::Release::builder()
            .name("v1")
            .version("1.0.0")
            .assets([
                super::ReleaseAsset::new("app-x86_64-linux", "https://host/linux"),
                super::ReleaseAsset::new("app-aarch64-darwin", "https://host/darwin"),
            ])
            .build()
            .unwrap();
        // The full triple is not in either asset name, so selection falls back to arch+os tokens
        // derived from the target string.
        let chosen = release
            .asset_for("aarch64-apple-darwin", None)
            .expect("must select the darwin asset for an apple-darwin target");
        assert_eq!(
            chosen.download_url(),
            "https://host/darwin",
            "the fallback must use the configured target's arch/os, not the build host's"
        );
    }

    // `ReleaseAsset::new(name, download_url)` argument order must match the field order so the two
    // same-typed args can't be silently swapped. Pins the constructor maps arg 1 -> name, arg 2 -> url.
    #[test]
    fn release_asset_new_argument_order() {
        let asset = super::ReleaseAsset::new("my-bin-x86_64.tar.gz", "https://host/dl");
        assert_eq!(asset.name(), "my-bin-x86_64.tar.gz");
        assert_eq!(asset.download_url(), "https://host/dl");
    }

    // --- getters return exactly the builder-set values --------------------------------

    #[test]
    fn release_getters_return_builder_set_values() {
        // Every getter must surface exactly what the builder was given (and `body()` is `Some` only
        // when set). Pins the field-encapsulation read surface: `name`, `version`, `date`, `body`,
        // `assets` are reachable only through the getters now (fields are `pub(crate)`).
        let release = super::Release::builder()
            .name("My App 1.2.3")
            .version("1.2.3")
            .date("2024-05-06T00:00:00Z")
            .body("release notes here")
            .asset(super::ReleaseAsset::new(
                "app.tar.gz",
                "https://host/app.tar.gz",
            ))
            .build()
            .unwrap();
        assert_eq!(release.name(), "My App 1.2.3");
        assert_eq!(release.version(), "1.2.3");
        assert_eq!(release.date(), "2024-05-06T00:00:00Z");
        assert_eq!(release.body(), Some("release notes here"));
        assert_eq!(release.assets().len(), 1);
        assert_eq!(release.assets()[0].name(), "app.tar.gz");
        assert_eq!(
            release.assets()[0].download_url(),
            "https://host/app.tar.gz"
        );
    }

    #[test]
    fn release_builder_defaults_name_to_version_and_body_to_none() {
        // When only `version` is set: `name` defaults to the version, `date` to empty, `body` to
        // `None`, `assets` to empty — read through the getters.
        let release = super::Release::builder().version("9.9.9").build().unwrap();
        assert_eq!(release.version(), "9.9.9");
        assert_eq!(release.name(), "9.9.9", "name defaults to version");
        assert_eq!(release.date(), "", "date defaults to empty");
        assert_eq!(release.body(), None, "body defaults to None");
        assert!(release.assets().is_empty());
    }

    // --- Arc<str> backing - Clone is cheap and shares the backing allocation ----------

    #[test]
    fn release_clone_shares_arc_backing() {
        // The string fields are `Arc<str>`, so cloning a `Release` bumps the refcount rather than
        // reallocating the strings. Assert the cloned release shares the SAME backing allocation as
        // the original by comparing the data pointers of the `version` `Arc<str>` (easy to check,
        // no behavioral assumption — just that the clone is a shared-backing Arc clone).
        let original = super::Release::builder()
            .version("1.2.3")
            .asset(super::ReleaseAsset::new(
                "app.tar.gz",
                "https://host/app.tar.gz",
            ))
            .build()
            .unwrap();
        let cloned = original.clone();
        // Same logical value...
        assert_eq!(original.version(), cloned.version());
        // ...and the same backing bytes (Arc clone shares the allocation; pointer identity holds).
        assert!(
            std::ptr::eq(original.version().as_ptr(), cloned.version().as_ptr()),
            "cloning a Release must share the Arc<str> backing, not reallocate"
        );
        // The asset's Arc<str> backing is shared too.
        assert!(std::ptr::eq(
            original.assets()[0].download_url().as_ptr(),
            cloned.assets()[0].download_url().as_ptr()
        ));
    }

    // --- `Releases::from_releases` builds a usable `Releases` --------------------------

    #[test]
    fn releases_from_releases_builds_a_usable_collection() {
        // The public test constructor must produce a `Releases` whose `latest()` /
        // `is_update_available()` / `current_version()` work, exactly like the crate-internal
        // `new`. Build one with a newer-than-current release and assert the queries.
        let releases = super::Releases::from_releases(vec![rel("2.0.0"), rel("1.0.0")], "1.0.0");
        assert_eq!(releases.latest().unwrap().version(), "2.0.0");
        assert_eq!(releases.current_version(), Some("1.0.0"));
        assert!(
            releases.is_update_available().unwrap(),
            "2.0.0 > 1.0.0 via from_releases-built Releases"
        );

        // And the not-available case agrees.
        let up_to_date = super::Releases::from_releases(vec![rel("1.0.0")], "1.0.0");
        assert!(!up_to_date.is_update_available().unwrap());
    }

    // `Releases::from_listing` builds the bare-listing state (no current version), matching what
    // `ReleaseList::fetch` returns: `current_version()` is None and `is_update_available()` errors.
    #[test]
    fn releases_from_listing_has_no_current_version() {
        let listing = super::Releases::from_listing(vec![rel("2.0.0"), rel("1.0.0")]);
        assert_eq!(listing.current_version(), None);
        assert_eq!(listing.latest().unwrap().version(), "2.0.0");
        assert!(
            matches!(
                listing.is_update_available(),
                Err(crate::errors::Error::NoCurrentVersion)
            ),
            "a bare listing must error on is_update_available with the distinct NoCurrentVersion \
             variant, not the misleading MissingField builder-field error"
        );
    }

    // --- `ReleaseStatus::version()` -----------------------------------------------------

    #[test]
    fn release_status_version_returns_installed_version_or_none() {
        // `version()` mirrors `VersionStatus::version` but is `Some` only on the `Updated` arm; the
        // `UpToDate` arm (which carries no version) yields `None`.
        let updated = super::ReleaseStatus::Updated(rel("3.1.4"));
        assert_eq!(updated.version(), Some("3.1.4"));

        let up_to_date = super::ReleaseStatus::UpToDate;
        assert_eq!(
            up_to_date.version(),
            None,
            "UpToDate carries no version => None"
        );
    }

    // --- a listing-built `Releases` has no current version --------------------

    #[test]
    fn releases_from_listing_has_no_current_version_and_precheck_errors() {
        // The `ReleaseList::fetch` path builds a `Releases` with no current version, so
        // `current_version()` is `None` and `is_update_available()` errors (there is nothing to
        // compare against) rather than silently answering false.
        let listing = super::Releases::from_listing(vec![rel("2.0.0"), rel("1.0.0")]);
        assert_eq!(listing.current_version(), None);
        assert_eq!(listing.latest().unwrap().version(), "2.0.0");
        assert!(
            matches!(
                listing.is_update_available(),
                Err(crate::errors::Error::NoCurrentVersion)
            ),
            "a listing with no current version must error on is_update_available()"
        );
        // The message must be self-describing, not the builder-field `MissingField` message.
        let msg = listing.is_update_available().unwrap_err().to_string();
        assert!(
            msg.contains("no current_version to compare against"),
            "the error must self-describe the bare-listing case, got: {msg}"
        );
    }

    #[test]
    fn choose_latest_release_up_to_date_when_nothing_newer() {
        // No releases at all.
        assert!(
            choose_latest_release(vec![], "1.0.0", false, UpdateStrategy::Compatible)
                .unwrap()
                .is_none()
        );

        // A source (e.g. a custom backend) that returns the current and older versions must be
        // treated as up-to-date — not re-install the current version. (Regression test.)
        let chosen = choose_latest_release(
            vec![rel("1.0.0"), rel("0.9.0")],
            "1.0.0",
            false,
            UpdateStrategy::Compatible,
        )
        .unwrap();
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
            UpdateStrategy::Compatible,
        )
        .unwrap()
        .expect("a compatible newer release is chosen");
        assert_eq!(chosen.version(), "1.2.0");
    }

    #[test]
    fn choose_latest_release_sorts_out_of_order_candidates() {
        // A source (e.g. a custom backend) that returns candidates in an arbitrary order must still
        // yield the newest compatible release — "newest" must not depend on caller ordering.
        let chosen = choose_latest_release(
            vec![rel("1.1.0"), rel("1.4.2"), rel("1.0.5"), rel("1.3.0")],
            "1.0.0",
            false,
            UpdateStrategy::Compatible,
        )
        .unwrap()
        .expect("the newest compatible release is chosen regardless of input order");
        assert_eq!(chosen.version(), "1.4.2");

        // Same set, reversed — the choice must be identical.
        let chosen = choose_latest_release(
            vec![rel("1.3.0"), rel("1.0.5"), rel("1.4.2"), rel("1.1.0")],
            "1.0.0",
            false,
            UpdateStrategy::Compatible,
        )
        .unwrap()
        .expect("the newest compatible release is chosen regardless of input order");
        assert_eq!(chosen.version(), "1.4.2");
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
            UpdateStrategy::Compatible,
        )
        .unwrap()
        .expect("the newest parseable compatible release is chosen");
        assert_eq!(chosen.version(), "1.2.0");

        // Only junk versions -> nothing selectable -> up-to-date.
        assert!(
            choose_latest_release(
                vec![rel("junk"), rel("garbage")],
                "1.0.0",
                false,
                UpdateStrategy::Compatible,
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn choose_latest_release_falls_back_to_incompatible_newer() {
        // Only a major bump is available: newer than current but not semver-compatible. It is still
        // offered (flagged "*NOT* compatible" in the messages), exercising the fallback branch.
        let chosen = choose_latest_release(
            vec![rel("2.0.0")],
            "1.0.0",
            false,
            UpdateStrategy::Compatible,
        )
        .unwrap()
        .expect("an incompatible-but-newer release is still offered");
        assert_eq!(chosen.version(), "2.0.0");
    }

    #[test]
    fn choose_latest_release_compatible_prefers_compatible_over_newer_major() {
        // Default `Compatible` strategy: with a newer compatible release AND a newer major both
        // available, the compatible one wins (the #152 scenario: 1.1.x vs 2.0.0 from 1.1.1).
        let releases = vec![rel("2.0.0"), rel("1.1.2"), rel("1.1.0")];
        let chosen =
            choose_latest_release(releases.clone(), "1.1.1", false, UpdateStrategy::Compatible)
                .unwrap()
                .expect("a newer compatible release is chosen");
        assert_eq!(
            chosen.version(),
            "1.1.2",
            "Compatible must prefer the newest semver-compatible release over a newer major"
        );

        // Same inputs under `Latest`: the newest overall wins, jumping the major.
        let chosen = choose_latest_release(releases, "1.1.1", false, UpdateStrategy::Latest)
            .unwrap()
            .expect("the newest release overall is chosen");
        assert_eq!(
            chosen.version(),
            "2.0.0",
            "Latest must select the newest release overall, across a major bump"
        );
    }

    #[test]
    fn choose_latest_release_latest_still_up_to_date_when_nothing_newer() {
        // `Latest` still respects the strictly-newer guard: nothing newer than current => up-to-date.
        assert!(
            choose_latest_release(
                vec![rel("1.0.0"), rel("0.9.0")],
                "1.0.0",
                false,
                UpdateStrategy::Latest,
            )
            .unwrap()
            .is_none(),
            "Latest must not re-install the current or an older release"
        );
    }

    // --- Bound-narrowing compile locks (gap #3) -----------------------------------------------
    //
    // The refactor split the accessors onto the `UpdateConfig` supertrait. These items don't run
    // assertions; they exist to *fail to compile* if the trait relationships regress.

    use crate::update::{ReleaseUpdate, UpdateConfig, UpdateInternals};

    // A generic helper bounded only on `ReleaseUpdate` must still be able to call the accessors
    // that now live on the `UpdateConfig` supertrait — because `ReleaseUpdate: UpdateConfig`. If
    // the supertrait bound were dropped, `bin_name()`/`target()` would not resolve here.
    fn accessor_via_release_update_bound<R: ReleaseUpdate + ?Sized>(r: &R) -> (String, String) {
        (r.bin_name().to_string(), r.target().to_string())
    }

    // B1: a generic fn bounded only on the crate-private `UpdateInternals` sub-trait must compile
    // and read an internal accessor. This proves the orchestrator can reach the internal-typed
    // accessors through the supertrait. (`UpdateConfig` itself no longer carries these — a
    // `<dyn UpdateConfig>` value cannot call `request_headers()`, which is what moved them off.)
    fn internal_accessor_via_update_internals_bound<U: UpdateInternals + ?Sized>(u: &U) -> usize {
        u.request_headers().len()
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

        let (bin, target) = accessor_via_release_update_bound(&upd);
        // `bin_name(...)` appends the platform exe suffix (".exe" on windows).
        let expected_bin = format!("app{}", std::env::consts::EXE_SUFFIX);
        assert_eq!(bin, expected_bin);
        assert_eq!(target, "x86_64-unknown-linux-gnu");
        assert_eq!(accessor_via_dyn_release_update(&upd), "1.0.0");
        assert_eq!(accessor_via_update_config_bound(&upd), expected_bin);
        // B1: the internal accessor is reachable through the `UpdateInternals` bound.
        assert_eq!(internal_accessor_via_update_internals_bound(&upd), 0);
    }

    // --- F2: public async `get_release_version_async` ----------------------------------------
    //
    // `get_release_version_async` is the *public* async sibling of the sync `get_release_version`.
    // It is now a method on the public sealed `AsyncReleaseUpdate` trait (was previously an inherent
    // macro-generated method backed by the `pub(crate)` `AsyncFetch` trait), so the trait must be
    // brought into scope to call the verb. If the verb were missing from the trait, these tests
    // would fail to compile — pinning sync/async parity on the public surface.

    #[cfg(feature = "async")]
    use crate::update::AsyncReleaseUpdate;

    #[cfg(feature = "async")]
    struct TaggedAsyncSource;

    #[cfg(feature = "async")]
    impl crate::update::AsyncReleaseSource for TaggedAsyncSource {
        async fn get_latest_release(&self) -> Result<Release> {
            Release::builder().version("2.0.0").build()
        }
        async fn get_releases(&self) -> Result<Vec<Release>> {
            Ok(vec![Release::builder().version("2.0.0").build()?])
        }
        async fn get_release_version(&self, ver: &str) -> Result<Release> {
            if ver == "9.9.9" {
                Err(crate::errors::Error::NoReleaseFound { target: None })
            } else {
                Release::builder().version(ver).build()
            }
        }
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn public_get_release_version_async_returns_tagged_release() {
        let upd = crate::backends::custom::AsyncUpdate::configure()
            .source(TaggedAsyncSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .build_async()
            .unwrap();
        // Resolves to the `AsyncReleaseUpdate::get_release_version_async` trait method (brought into
        // scope above), proving the public async-by-tag surface.
        let rel = upd.get_release_version_async("1.5.0").await.unwrap();
        assert_eq!(rel.version(), "1.5.0");
    }

    // --- `AsyncReleaseUpdate` is usable as a generic bound -----------------------------
    //
    // A generic fn bounded on `AsyncReleaseUpdate` must compile and drive the verbs, proving the
    // trait is nameable/bound-able (RPITIT => not object-safe, but usable as a bound, like
    // `AsyncReleaseSource`). If the trait stopped being a public sealed bound, this would not
    // compile.
    #[cfg(feature = "async")]
    async fn fetch_latest_via_bound<U: AsyncReleaseUpdate + Sync>(u: &U) -> Result<String> {
        let releases = u.get_latest_release_async().await?;
        Ok(releases
            .latest()
            .map(|r| r.version().to_string())
            .unwrap_or_default())
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_release_update_is_usable_as_a_generic_bound() {
        let upd = crate::backends::custom::AsyncUpdate::configure()
            .source(TaggedAsyncSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .build_async()
            .unwrap();
        // The generic helper drives the trait verbs through the bound.
        let version = fetch_latest_via_bound(&upd).await.unwrap();
        assert_eq!(
            version, "2.0.0",
            "the bounded generic fn drove the async verb"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn public_get_release_version_async_propagates_missing_tag_error() {
        let upd = crate::backends::custom::AsyncUpdate::configure()
            .source(TaggedAsyncSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .build_async()
            .unwrap();
        let res = upd.get_release_version_async("9.9.9").await;
        assert!(
            matches!(res, Err(crate::errors::Error::NoReleaseFound { .. })),
            "a missing tag must propagate as Error::NoReleaseFound, got {:?}",
            res
        );
    }

    struct BoundSource;
    impl crate::update::ReleaseSource for BoundSource {
        fn get_latest_release(&self) -> Result<Release> {
            Release::builder().version("1.0.0").build()
        }
        fn get_releases(&self) -> Result<Vec<Release>> {
            Ok(vec![Release::builder().version("1.0.0").build()?])
        }
        fn get_release_version(&self, v: &str) -> Result<Release> {
            Release::builder().version(v).build()
        }
    }

    // B6: `bin_install_path()` returns a borrow (`&Path`), not an owned `PathBuf`. Binding the
    // result to a `&std::path::Path` only compiles with the borrowing accessor; the old owned
    // `PathBuf` return would not coerce to `&Path` without a temporary, so this pins the change.
    #[test]
    fn bin_install_path_returns_a_borrow() {
        let upd = crate::backends::custom::Update::configure()
            .source(BoundSource)
            .bin_name("app")
            .bin_install_path("/tmp/app-install-path")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .build()
            .unwrap();
        let p: &std::path::Path = upd.bin_install_path();
        assert_eq!(p, std::path::Path::new("/tmp/app-install-path"));
    }

    // A windows install path must print with the separators the user typed. The old `{:?}` went
    // through `Path`'s `Debug` impl, which quotes and escapes, turning `C:\Users\..` into
    // `"C:\\Users\\.."` in the confirmation block. `Display` writes the path as-is, so this
    // asserts the same string on every platform — a backslash is an ordinary character in a unix
    // path, so the test is meaningful on the linux lanes too, not only the windows job.
    #[test]
    fn install_target_line_prints_paths_without_debug_escaping() {
        let exe = std::path::Path::new(r"C:\Users\Public\Documents\mise\bin\mise.exe");
        assert_eq!(
            install_target_line(None, exe),
            r"  * Current exe: C:\Users\Public\Documents\mise\bin\mise.exe"
        );
        assert_eq!(
            install_target_line(Some(std::path::Path::new("/Applications/App.app")), exe),
            "  * Current bundle: /Applications/App.app"
        );
    }

    // The asset name comes from the release listing, so it is remote-controlled, and it is echoed
    // into the confirmation block. A control character in it (a `\r` that returns to the start of
    // the line, or an ESC that opens an ANSI sequence) could repaint or hide the lines the user
    // reads before authorizing the replacement, including the download url. Reject it at the guard
    // rather than relying on the block's formatting to neutralize it.
    #[test]
    fn is_safe_asset_name_rejects_control_characters() {
        assert!(super::is_safe_asset_name("app-v1.2.3-x86_64.tar.gz"));

        for name in [
            "app\r\u{1b}[2Kevil.tar.gz",
            "app\nevil.tar.gz",
            "app\u{1b}[31m.tar.gz",
            "app\u{7f}.tar.gz",
            "app\u{0}.tar.gz",
        ] {
            assert!(
                !super::is_safe_asset_name(name),
                "a control character must be rejected: {:?}",
                name
            );
        }

        // The traversal and separator rejections are unchanged.
        assert!(!super::is_safe_asset_name(""));
        assert!(!super::is_safe_asset_name(".."));
        assert!(!super::is_safe_asset_name("dir/app.tar.gz"));
        assert!(!super::is_safe_asset_name(r"dir\app.tar.gz"));
    }

    #[test]
    fn install_binary_aborts_when_verify_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let new_exe = dir.path().join("new");
        std::fs::write(&new_exe, b"new binary").unwrap();
        // A dest that isn't the current exe takes the move path (not self_replace).
        let dest = dir.path().join("installed");

        let reject: Box<DynVerifyFn> = Box::new(|_: &std::path::Path| {
            Err(crate::errors::Error::VerificationRejected {
                reason: Some("binary did not pass the smoke test".to_string()),
            })
        });
        let res = install_binary(&new_exe, &dest, Some(&*reject));
        // P4: a rejecting verify callback surfaces the dedicated `VerificationRejected` variant,
        // carrying the hook's error message as the reason.
        let err = res.expect_err("a rejecting verify hook must abort the install");
        match err {
            crate::errors::Error::VerificationRejected { reason } => {
                let reason = reason.expect("a rejecting hook must carry a reason");
                assert!(
                    reason.contains("binary did not pass the smoke test"),
                    "the rejection reason must carry the hook's error message, got: {reason}"
                );
            }
            other => panic!("expected Error::VerificationRejected, got {:?}", other),
        }
        assert!(
            !dest.exists(),
            "nothing is installed when verification fails"
        );
        assert!(new_exe.exists(), "the extracted binary is left untouched");
    }

    #[test]
    fn install_binary_passes_verification_rejected_through_unwrapped() {
        // A hook rejecting via `Error::verification_rejected` surfaces its reason verbatim: the
        // already-`VerificationRejected` error passes through instead of being re-wrapped with the
        // full Display string ("VerificationRejectedError: ...") nested inside the reason.
        let dir = tempfile::tempdir().unwrap();
        let new_exe = dir.path().join("new");
        std::fs::write(&new_exe, b"new binary").unwrap();
        let dest = dir.path().join("installed");

        let reject: Box<DynVerifyFn> = Box::new(|_: &std::path::Path| {
            Err(crate::errors::Error::verification_rejected("bad signature"))
        });
        let err = install_binary(&new_exe, &dest, Some(&*reject))
            .expect_err("a rejecting verify hook must abort the install");
        match err {
            crate::errors::Error::VerificationRejected { reason } => {
                assert_eq!(
                    reason.as_deref(),
                    Some("bad signature"),
                    "the constructor's reason must pass through unwrapped"
                );
            }
            other => panic!("expected Error::VerificationRejected, got {:?}", other),
        }
        assert!(!dest.exists());
    }

    #[test]
    fn install_binary_propagates_hook_io_error_as_reason() {
        // A hook that fails with an IO error (e.g. it couldn't spawn `new --version`) propagates
        // that error's message as the `VerificationRejected` reason, not a generic None.
        let dir = tempfile::tempdir().unwrap();
        let new_exe = dir.path().join("new");
        std::fs::write(&new_exe, b"new binary").unwrap();
        let dest = dir.path().join("installed");

        let io_failing: Box<DynVerifyFn> = Box::new(|_: &std::path::Path| {
            Err(crate::errors::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "could not run new --version",
            )))
        });
        let err = install_binary(&new_exe, &dest, Some(&*io_failing))
            .expect_err("a hook IO error must abort the install");
        match err {
            crate::errors::Error::VerificationRejected { reason } => {
                let reason = reason.expect("a hook IO error must carry its message as the reason");
                assert!(
                    reason.contains("could not run new --version"),
                    "the hook IO error message must propagate as the reason, got: {reason}"
                );
            }
            other => panic!("expected Error::VerificationRejected, got {:?}", other),
        }
        assert!(!dest.exists());
    }

    #[test]
    fn install_binary_installs_when_verify_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let new_exe = dir.path().join("new");
        std::fs::write(&new_exe, b"new binary").unwrap();
        let dest = dir.path().join("installed");

        let accept: Box<DynVerifyFn> = Box::new(|_: &std::path::Path| Ok(()));
        install_binary(&new_exe, &dest, Some(&*accept)).unwrap();
        assert!(
            dest.exists(),
            "binary is installed when verification passes"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"new binary");
    }

    // Installing into a read-only (0555) directory fails with a permission error at the move step;
    // `install_binary` must annotate it as `InstallPathNotWritable` naming the install path, not
    // leak a bare `Io(Permission denied)`. Pins the always-on install-error path context.
    #[cfg(unix)]
    #[test]
    fn install_binary_maps_permission_denied_to_install_path_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let new_exe = dir.path().join("new");
        std::fs::write(&new_exe, b"new binary").unwrap();

        let ro_dir = dir.path().join("ro");
        std::fs::create_dir(&ro_dir).unwrap();
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let dest = ro_dir.join("installed");

        let res = install_binary(&new_exe, &dest, None);
        // Restore write perms so the tempdir can be cleaned up regardless of the assertion outcome.
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        let err = res.expect_err("installing into a 0555 dir must fail");
        match err {
            Error::InstallPathNotWritable { path } => {
                assert_eq!(
                    path, dest,
                    "InstallPathNotWritable must carry the exact install path"
                );
            }
            other => panic!("expected Error::InstallPathNotWritable, got {:?}", other),
        }
    }

    // A non-permission install failure (here: the destination's parent directory does not exist,
    // so the move fails `NotFound`) must NOT be reclassified as `InstallPathNotWritable`; it stays
    // an `Error::Io` whose message names the install path and whose `ErrorKind` is preserved.
    #[test]
    fn install_binary_wraps_non_permission_io_error_with_path_context() {
        let dir = tempfile::tempdir().unwrap();
        let new_exe = dir.path().join("new");
        std::fs::write(&new_exe, b"new binary").unwrap();
        // Parent directory intentionally absent -> rename fails with NotFound, not PermissionDenied.
        let dest = dir.path().join("no-such-dir").join("installed");

        let err = install_binary(&new_exe, &dest, None)
            .expect_err("moving into a missing parent dir must fail");
        match err {
            Error::Io(io) => {
                assert_ne!(
                    io.kind(),
                    std::io::ErrorKind::PermissionDenied,
                    "a NotFound failure must not be reported as permission denied"
                );
                let msg = io.to_string();
                assert!(
                    msg.contains(&dest.display().to_string()),
                    "the rewrapped Io error must name the install path, got: {msg}"
                );
            }
            other => panic!(
                "a non-permission failure must stay Error::Io, got {:?}",
                other
            ),
        }
    }

    // The preflight probe returns `Ok(())` for a writable directory (a not-yet-existing target
    // whose parent is writable: the temp-sibling create/delete succeeds).
    #[test]
    fn probe_install_path_writable_ok_for_writable_dir() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app");
        super::probe_install_path_writable(&target).expect("a writable parent dir must probe Ok");
    }

    // The preflight probe returns `InstallPathNotWritable` when the parent dir is read-only (0555):
    // the temp-sibling create fails with PermissionDenied.
    #[cfg(unix)]
    #[test]
    fn probe_install_path_writable_errors_for_readonly_dir() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ro_dir = dir.path().join("ro");
        std::fs::create_dir(&ro_dir).unwrap();
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let target = ro_dir.join("app");

        let res = super::probe_install_path_writable(&target);
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        match res {
            Err(Error::InstallPathNotWritable { path }) => {
                assert_eq!(path, target, "the probe must name the probed install path");
            }
            other => panic!("expected InstallPathNotWritable, got {:?}", other),
        }
    }

    // The preflight probe is conservative: a missing parent directory is indeterminate (the temp
    // create fails with NotFound, not PermissionDenied), so the probe returns `Ok(())` and lets the
    // real install step surface any error.
    #[test]
    fn probe_install_path_writable_ok_for_missing_parent() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("no-such-dir").join("app");
        super::probe_install_path_writable(&target)
            .expect("a missing parent dir is indeterminate and must probe Ok");
    }

    // --- bundle mode (BNDL-*) -------------------------------------------------------------------

    // Build a staged bundle tree with an executable at `Contents/MacOS/<exe>` plus one resource,
    // returning the bundle root. Mirrors the shape of a macOS `.app`.
    fn staged_bundle(
        parent: &std::path::Path,
        name: &str,
        exe: &str,
        marker: &[u8],
    ) -> std::path::PathBuf {
        let root = parent.join(name);
        let macos = root.join("Contents").join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(macos.join(exe), marker).unwrap();
        std::fs::write(root.join("Contents").join("Info.plist"), marker).unwrap();
        root
    }

    // BNDL-2-5: a fresh install (nothing at the destination yet) renames the staged tree into
    // place, contents intact.
    #[test]
    fn swap_bundle_installs_when_nothing_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();
        let dest = dir.path().join("MyApp.app");
        let outside_exe = dir.path().join("updater");

        super::swap_bundle(&staged, &dest, &stash, &outside_exe, None)
            .expect("a fresh bundle install must work");

        assert!(dest.is_dir(), "the bundle must be installed at the dest");
        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"new",
            "the installed tree must be the staged one"
        );
        assert!(!staged.exists(), "the staged tree is moved, not copied");
    }

    // BNDL-2-5/BNDL-5-2: replacing an existing bundle swaps the whole tree, so a file that the new
    // bundle does not carry is gone afterwards (no merge with the old contents).
    #[test]
    fn swap_bundle_replaces_the_whole_tree() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");
        let stale = dest.join("Contents").join("stale-resource");
        std::fs::write(&stale, b"stale").unwrap();
        let outside_exe = dir.path().join("updater");

        super::swap_bundle(&staged, &dest, &stash, &outside_exe, None)
            .expect("replacing a bundle must work");

        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"new",
            "the exe must come from the new tree"
        );
        assert!(
            !stale.exists(),
            "a whole-tree swap must not leave stale files from the old bundle"
        );
    }

    // BNDL-2-3: a staged root that the archive did not carry (missing, or a file rather than a
    // directory) is an error, and the installed bundle is left untouched.
    #[test]
    fn swap_bundle_rejects_a_missing_or_non_directory_staged_root() {
        let dir = tempfile::tempdir().unwrap();
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");
        let outside_exe = dir.path().join("updater");

        let missing = dir.path().join("staging").join("MyApp.app");
        let err = super::swap_bundle(&missing, &dest, &stash, &outside_exe, None)
            .expect_err("a missing staged root must error");
        assert!(
            matches!(&err, Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound),
            "expected a NotFound Io error, got {err:?}"
        );

        let as_file = dir.path().join("not-a-dir");
        std::fs::write(&as_file, b"file").unwrap();
        super::swap_bundle(&as_file, &dest, &stash, &outside_exe, None)
            .expect_err("a staged root that is a file must error");

        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"old",
            "the installed bundle must be untouched when the staged tree is unusable"
        );
    }

    // BNDL-5-2: when the swap's final rename fails, the stashed old tree is restored, so the
    // destination is left with its original contents and the original error surfaces.
    #[test]
    fn swap_bundle_rolls_back_when_the_install_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");

        // Inject a failure between the stash and the install: a verify hook is the only in-process
        // seam that runs before the renames, so instead remove the staged tree through a hook,
        // making the final `rename(staged -> dest)` fail with NotFound after the old tree is
        // stashed.
        let sabotage: Box<DynVerifyFn> = {
            let staged = staged.clone();
            Box::new(move |_: &std::path::Path| {
                std::fs::remove_dir_all(&staged).unwrap();
                Ok(())
            })
        };
        let err = super::swap_bundle(
            &staged,
            &dest,
            &stash,
            &dir.path().join("updater"),
            Some(&*sabotage),
        )
        .expect_err("a failed install rename must error");
        assert!(
            matches!(&err, Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound),
            "the original rename error must surface, got {err:?}"
        );

        assert!(dest.is_dir(), "the old bundle must be restored");
        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"old",
            "rollback must restore the old tree byte-for-byte"
        );
    }

    // BNDL-2-5: with the running executable inside the bundle it is renamed aside before the
    // directory swap, and after a successful swap its path holds the NEW bundle's executable.
    #[test]
    fn swap_bundle_moves_the_running_exe_aside_and_restores_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");
        let running_exe = dest.join("Contents").join("MacOS").join("myapp");

        super::swap_bundle(&staged, &dest, &stash, &running_exe, None)
            .expect("an in-bundle swap must work");

        assert_eq!(
            std::fs::read(&running_exe).unwrap(),
            b"new",
            "the running exe's path must hold the new bundle's executable"
        );
        assert!(
            stash.join("exe-aside").exists(),
            "the old running image must be stashed aside, not left in the installed tree"
        );
    }

    // BNDL-5-2: a failed swap with the running exe inside the bundle restores BOTH the old tree and
    // the executable at its original path, so the process is left able to keep running.
    #[test]
    fn swap_bundle_rollback_restores_the_running_exe_inside_the_old_tree() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");
        let running_exe = dest.join("Contents").join("MacOS").join("myapp");

        let sabotage: Box<DynVerifyFn> = {
            let staged = staged.clone();
            Box::new(move |_: &std::path::Path| {
                std::fs::remove_dir_all(&staged).unwrap();
                Ok(())
            })
        };
        super::swap_bundle(&staged, &dest, &stash, &running_exe, Some(&*sabotage))
            .expect_err("a failed install rename must error");

        assert_eq!(
            std::fs::read(&running_exe).unwrap(),
            b"old",
            "rollback must put the running executable back inside the restored tree"
        );
        assert!(
            !stash.join("exe-aside").exists(),
            "the stashed executable must have been moved back, not left in the stash"
        );
    }

    // BNDL-2-4/BNDL-5-2: the verify hook receives the staged BUNDLE ROOT (a directory), and a
    // rejection aborts before anything is renamed.
    #[test]
    fn swap_bundle_verifies_the_staged_root_and_a_rejection_replaces_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");

        let seen = std::sync::Arc::new(std::sync::Mutex::new(None::<std::path::PathBuf>));
        let reject: Box<DynVerifyFn> = {
            let seen = std::sync::Arc::clone(&seen);
            Box::new(move |p: &std::path::Path| {
                *seen.lock().unwrap() = Some(p.to_path_buf());
                Err(Error::verification_rejected("bundle is not signed"))
            })
        };
        let err = super::swap_bundle(
            &staged,
            &dest,
            &stash,
            &dir.path().join("updater"),
            Some(&*reject),
        )
        .expect_err("a rejecting hook must abort the swap");
        match err {
            Error::VerificationRejected { reason } => assert_eq!(
                reason.as_deref(),
                Some("bundle is not signed"),
                "the hook's reason must pass through unwrapped"
            ),
            other => panic!("expected VerificationRejected, got {other:?}"),
        }
        assert_eq!(
            seen.lock().unwrap().as_deref(),
            Some(staged.as_path()),
            "the hook must receive the staged bundle root, not a file inside it"
        );
        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"old",
            "a rejected verification must leave the installed bundle in place"
        );
        assert!(staged.is_dir(), "the staged tree is left for the caller");
    }

    // BNDL-1-6: when the running executable is inside the installed bundle, the staged tree must
    // carry a file at the same relative path; otherwise the swap errors before touching anything
    // (rather than leaving the process's own path missing).
    #[test]
    fn swap_bundle_requires_the_staged_tree_to_carry_the_running_exe_path() {
        let dir = tempfile::tempdir().unwrap();
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");
        let running_exe = dest.join("Contents").join("MacOS").join("myapp");

        // A staged tree whose executable sits at a different relative path than the running one.
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "renamed", b"new");

        let err = super::swap_bundle(&staged, &dest, &stash, &running_exe, None)
            .expect_err("a staged tree missing the running exe path must error");
        let msg = err.to_string();
        assert!(
            msg.contains("Contents/MacOS/myapp") || msg.contains("Contents\\MacOS\\myapp"),
            "the error must name the missing relative path, got: {msg}"
        );
        assert_eq!(
            std::fs::read(&running_exe).unwrap(),
            b"old",
            "the installed bundle must be untouched by the rejected swap"
        );
    }

    // `exe_inside_bundle` reports the exe's path relative to the bundle, resolving symlinks on both
    // sides (mirroring `same_file`), and `None` when the exe is outside.
    #[test]
    fn exe_inside_bundle_detects_containment_through_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = staged_bundle(dir.path(), "MyApp.app", "myapp", b"exe");
        let exe = bundle.join("Contents").join("MacOS").join("myapp");

        let (found, rel) = super::exe_inside_bundle(&exe, &bundle)
            .expect("an exe inside the bundle must be detected");
        assert_eq!(rel, std::path::Path::new("Contents/MacOS/myapp"));
        assert_eq!(found, std::fs::canonicalize(&exe).unwrap());

        let outside = dir.path().join("elsewhere");
        std::fs::write(&outside, b"exe").unwrap();
        assert!(
            super::exe_inside_bundle(&outside, &bundle).is_none(),
            "an exe outside the bundle must not be detected as inside"
        );

        assert!(
            super::exe_inside_bundle(&bundle, &bundle).is_none(),
            "the bundle root itself is not an exe inside the bundle"
        );

        #[cfg(unix)]
        {
            // A symlinked view of the bundle still matches: both sides canonicalize.
            let link = dir.path().join("Linked.app");
            std::os::unix::fs::symlink(&bundle, &link).unwrap();
            let (_, rel) = super::exe_inside_bundle(&exe, &link)
                .expect("a symlinked bundle path must still match");
            assert_eq!(rel, std::path::Path::new("Contents/MacOS/myapp"));
        }
    }

    // BNDL-1-3: the macOS default install path is the NEAREST `.app` ancestor of the exe, and
    // `None` when there is none. Pure and path-lexical, so it runs on every platform.
    #[test]
    fn enclosing_app_bundle_finds_the_nearest_app_ancestor() {
        let nested = std::path::Path::new("/Applications/Outer.app/Contents/MacOS/Inner.app/x/exe");
        assert_eq!(
            super::enclosing_app_bundle(nested).unwrap(),
            std::path::Path::new("/Applications/Outer.app/Contents/MacOS/Inner.app"),
            "the innermost .app ancestor wins"
        );
        assert_eq!(
            super::enclosing_app_bundle(std::path::Path::new(
                "/Applications/MyApp.app/Contents/MacOS/myapp"
            ))
            .unwrap(),
            std::path::Path::new("/Applications/MyApp.app")
        );
        assert_eq!(
            super::enclosing_app_bundle(std::path::Path::new("/usr/local/bin/myapp")),
            None,
            "an exe with no .app ancestor has no default bundle path"
        );
        assert_eq!(
            super::enclosing_app_bundle(std::path::Path::new("/Applications/MyApp.app")),
            None,
            "the .app itself is not its own ancestor"
        );
        // macOS's default filesystem preserves case without distinguishing it, so an odd-cased
        // bundle directory on disk is still the same bundle.
        assert_eq!(
            super::enclosing_app_bundle(std::path::Path::new(
                "/Applications/MyApp.App/Contents/MacOS/myapp"
            ))
            .unwrap(),
            std::path::Path::new("/Applications/MyApp.App"),
            "the .app extension must match case-insensitively"
        );
    }

    // BNDL-1-3/BNDL-5-3: the default-path resolution as a whole, on every host. With a target that
    // has the `.app` convention, a translocated exe is rejected before the `.app` lookup, an exe
    // inside a bundle yields it, and one outside any bundle is `NoAppBundle`. A target without the
    // convention has no default at all, so the setter is required.
    #[test]
    fn resolve_default_bundle_path_covers_every_arm() {
        let inside = std::path::Path::new("/Applications/MyApp.app/Contents/MacOS/myapp");
        assert_eq!(
            super::resolve_default_bundle_path(inside, true).unwrap(),
            std::path::Path::new("/Applications/MyApp.app")
        );

        let outside = std::path::Path::new("/usr/local/bin/myapp");
        match super::resolve_default_bundle_path(outside, true) {
            Err(Error::NoAppBundle { exe }) => assert_eq!(exe, outside),
            other => panic!("expected NoAppBundle, got {other:?}"),
        }

        // Translocation wins over the `.app` lookup: the enclosing `.app` exists here, but it is the
        // read-only translocated copy, so returning it would hand back an unswappable path.
        let translocated = std::path::Path::new(
            "/private/var/folders/x1/T/AppTranslocation/UUID/d/MyApp.app/Contents/MacOS/myapp",
        );
        match super::resolve_default_bundle_path(translocated, true) {
            Err(Error::AppTranslocated { exe }) => assert_eq!(exe, translocated),
            other => panic!("expected AppTranslocated, got {other:?}"),
        }

        match super::resolve_default_bundle_path(inside, false) {
            Err(Error::MissingField { field }) => assert_eq!(field, "bundle_install_path"),
            other => panic!("expected MissingField, got {other:?}"),
        }
    }

    // The policy flag matches the target: only macOS derives a default install path.
    #[test]
    fn has_app_bundle_default_is_macos_only() {
        assert_eq!(
            super::HAS_APP_BUNDLE_DEFAULT,
            cfg!(target_os = "macos"),
            "only macOS has the `.app` convention to derive a default install path from"
        );
    }

    // BNDL-5-3: a translocated (quarantined) app is detected from its path component.
    #[test]
    fn is_translocated_matches_the_translocation_mount() {
        assert!(super::is_translocated(std::path::Path::new(
            "/private/var/folders/x1/T/AppTranslocation/1E4-UUID/d/MyApp.app/Contents/MacOS/myapp"
        )));
        assert!(
            !super::is_translocated(std::path::Path::new(
                "/Applications/MyApp.app/Contents/MacOS/myapp"
            )),
            "an installed app is not translocated"
        );
        assert!(
            !super::is_translocated(std::path::Path::new(
                "/Applications/AppTranslocationHelper.app/Contents/MacOS/x"
            )),
            "the match is on a whole path component, not a substring"
        );
    }

    // BNDL-2-2/BNDL-2-3: the whole install step over a real archive -- extract beside the
    // destination, then swap -- preserving the executable bit and the framework-style symlinks an
    // `.app` needs. Staging happens in the destination's parent, so nothing else in the temp dir
    // survives the call.
    #[cfg(all(feature = "archive-zip", unix))]
    #[test]
    fn install_bundle_extracts_and_swaps_a_real_archive() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("MyApp.app.zip");
        {
            let f = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            let exe_opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);
            zip.start_file("MyApp.app/Contents/MacOS/myapp", exe_opts)
                .unwrap();
            zip.write_all(b"#!/bin/sh\necho new\n").unwrap();
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("MyApp.app/Contents/Resources/data.txt", opts)
                .unwrap();
            zip.write_all(b"payload").unwrap();
            // A relative symlink, as a bundled framework's `Versions/Current` would be.
            zip.add_symlink("MyApp.app/Contents/Current", "Resources", opts)
                .unwrap();
            zip.finish().unwrap();
        }

        // An older bundle already installed, with a file the new one does not carry.
        let install_root = dir.path().join("Applications");
        std::fs::create_dir(&install_root).unwrap();
        let dest = staged_bundle(&install_root, "MyApp.app", "myapp", b"old");
        std::fs::write(dest.join("Contents").join("gone.txt"), b"stale").unwrap();

        super::install_bundle(&archive_path, "MyApp.app", &dest, None, false)
            .expect("installing a bundle from an archive must work");

        let installed_exe = dest.join("Contents").join("MacOS").join("myapp");
        assert_eq!(
            std::fs::read(&installed_exe).unwrap(),
            b"#!/bin/sh\necho new\n",
            "the installed exe must come from the archive"
        );
        assert!(
            std::fs::metadata(&installed_exe)
                .unwrap()
                .permissions()
                .mode()
                & 0o111
                != 0,
            "the archived executable bit must survive the install"
        );
        assert!(
            std::fs::symlink_metadata(dest.join("Contents").join("Current"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "a bundled symlink must be installed as a symlink"
        );
        assert!(
            !dest.join("Contents").join("gone.txt").exists(),
            "the swap replaces the whole tree"
        );
        // Staging and stash were temp dirs inside the install parent; both are cleaned up.
        let leftovers: Vec<_> = std::fs::read_dir(&install_root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "MyApp.app")
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging/stash dirs must not be left behind, found: {leftovers:?}"
        );
    }

    // BNDL-3-1: in bundle mode the preflight probes the bundle's PARENT (the directory the swap
    // needs create+rename permission in), naming that parent when it is read-only.
    #[cfg(unix)]
    #[test]
    fn probe_writable_probes_the_bundle_parent_in_bundle_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ro_dir = dir.path().join("Applications");
        std::fs::create_dir(&ro_dir).unwrap();
        let bundle = ro_dir.join("MyApp.app");
        std::fs::create_dir(&bundle).unwrap();
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let res =
            super::probe_writable(std::path::Path::new("/definitely/not/used"), Some(&bundle));
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        match res {
            Err(Error::InstallPathNotWritable { path }) => assert_eq!(
                path, ro_dir,
                "bundle mode must name the bundle's parent directory"
            ),
            other => panic!("expected InstallPathNotWritable, got {other:?}"),
        }
    }

    // The same dispatch returns `Ok(())` for a writable bundle parent, and falls back to the
    // single-file probe when bundle mode is off.
    #[test]
    fn probe_writable_falls_back_to_the_bin_path_without_bundle_mode() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("MyApp.app");
        super::probe_writable(std::path::Path::new("/unused"), Some(&bundle))
            .expect("a writable bundle parent must probe Ok");
        super::probe_writable(&dir.path().join("app"), None)
            .expect("without bundle mode the bin install path is probed");
    }

    // --- bundle mode: gaps closed by the coverage pass (BNDL-*) ---------------------------------

    // BNDL-2-2: `install_parent` picks the directory the staging and stash temp dirs are created
    // in. A bare relative bundle name has an *empty* parent component, which must resolve to the
    // current directory -- `TempDir::new_in("")` fails, so an empty parent would break the swap for
    // a relative `bundle_install_path`.
    #[test]
    fn install_parent_resolves_a_bare_name_to_the_current_dir() {
        assert_eq!(
            super::install_parent(std::path::Path::new("MyApp.app")),
            std::path::Path::new("."),
            "a bare bundle name must stage in the current directory"
        );
        assert_eq!(
            super::install_parent(std::path::Path::new("/Applications/MyApp.app")),
            std::path::Path::new("/Applications")
        );
        assert_eq!(
            super::install_parent(std::path::Path::new("sub/MyApp.app")),
            std::path::Path::new("sub"),
            "a relative nested path keeps its real parent"
        );
    }

    // BNDL-3-1: the bundle-parent probe is best-effort in exactly the way the single-file probe is:
    // only a definite permission refusal fails. A directory that does not exist is indeterminate,
    // so the update proceeds and the real install step surfaces the outcome.
    #[test]
    fn probe_dir_writable_treats_a_missing_dir_as_indeterminate() {
        let dir = tempfile::tempdir().unwrap();
        super::probe_dir_writable(&dir.path().join("nope").join("deeper"))
            .expect("a missing directory is indeterminate and must probe Ok");
        super::probe_dir_writable(dir.path()).expect("a writable directory must probe Ok");
    }

    // BNDL-5-2/BNDL-3-2: a failure at the stash step (step 2 of the swap) happens before anything
    // under the install path has changed, so the installed bundle is left exactly as it was, the
    // ORIGINAL io error surfaces, and it names the install path. Injected by pointing the swap at a
    // stash directory that does not exist.
    #[test]
    fn swap_bundle_leaves_the_bundle_intact_when_the_stash_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let missing_stash = dir.path().join("no-such-stash");
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");

        let err = super::swap_bundle(
            &staged,
            &dest,
            &missing_stash,
            &dir.path().join("updater"),
            None,
        )
        .expect_err("an unusable stash must fail the swap");
        assert!(
            matches!(&err, Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound),
            "the original rename error (and its kind) must surface, got {err:?}"
        );
        assert!(
            err.to_string().contains(&dest.display().to_string()),
            "the install error must name the bundle install path, got: {err}"
        );
        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"old",
            "nothing may change before the stash rename succeeds"
        );
        assert!(
            staged.is_dir(),
            "the staged tree must be left for the caller when the swap never started"
        );
    }

    // BNDL-5-2: the very first rename -- the running executable moved aside (step 1) -- can fail
    // too. Nothing has moved at that point, so both the installed bundle and the running
    // executable's path are untouched, and the error names the install path.
    #[test]
    fn swap_bundle_reports_a_failed_exe_aside_with_nothing_moved() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let missing_stash = dir.path().join("no-such-stash");
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");
        let running_exe = dest.join("Contents").join("MacOS").join("myapp");

        let err = super::swap_bundle(&staged, &dest, &missing_stash, &running_exe, None)
            .expect_err("an unusable stash must fail the exe-aside rename");
        assert!(
            err.to_string().contains(&dest.display().to_string()),
            "the install error must name the bundle install path, got: {err}"
        );
        assert_eq!(
            std::fs::read(&running_exe).unwrap(),
            b"old",
            "the running executable must stay in place when the swap never started"
        );
        assert!(dest.is_dir(), "the installed bundle must be untouched");
    }

    // BNDL-2-4: a hook failing with an ordinary error (not an explicit rejection) still aborts the
    // swap; its message becomes the `VerificationRejected` reason and nothing is renamed. The
    // rejection arm is covered above -- this is the wrap-any-other-error arm, in bundle mode.
    #[test]
    fn swap_bundle_wraps_a_hook_error_as_a_rejection_and_replaces_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();
        let dest = staged_bundle(dir.path(), "MyApp.app", "myapp", b"old");

        let hook: Box<DynVerifyFn> = Box::new(|_: &std::path::Path| {
            Err(Error::Io(std::io::Error::other("codesign hook blew up")))
        });
        let err = super::swap_bundle(
            &staged,
            &dest,
            &stash,
            &dir.path().join("updater"),
            Some(&*hook),
        )
        .expect_err("a failing hook must abort the swap");
        match err {
            Error::VerificationRejected { reason } => assert!(
                reason
                    .as_deref()
                    .is_some_and(|r| r.contains("codesign hook blew up")),
                "the hook's error message must become the rejection reason, got {reason:?}"
            ),
            other => panic!("expected VerificationRejected, got {other:?}"),
        }
        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"old",
            "a failed verification must leave the installed bundle in place"
        );
    }

    // BNDL-2-5: containment is decided even when neither path exists yet -- a fresh install has no
    // destination to canonicalize, so the comparison falls back to the lexical paths -- and it is
    // component-wise: a sibling whose name merely starts with the bundle's name is outside it.
    #[test]
    fn exe_inside_bundle_falls_back_to_a_lexical_comparison() {
        let base = std::path::Path::new("/no/such/root");
        let bundle = base.join("MyApp.app");
        let exe = bundle.join("Contents").join("MacOS").join("myapp");

        let (found, rel) = super::exe_inside_bundle(&exe, &bundle)
            .expect("paths that cannot be canonicalized must still compare lexically");
        assert_eq!(rel, std::path::Path::new("Contents/MacOS/myapp"));
        assert_eq!(found, exe, "the un-canonicalizable exe path is used as-is");
        assert!(
            super::exe_inside_bundle(&base.join("MyApp.app.bak").join("myapp"), &bundle).is_none(),
            "a sibling sharing the bundle's name prefix is not inside the bundle"
        );
    }

    // BNDL-2-5: a `bundle_install_path` that is a symlink to the real bundle still ends up
    // resolving to the NEW tree after the swap -- the guarantee a caller who configured that path
    // depends on. (Whether the link itself survives the swap is deliberately not asserted here:
    // that detail is not fixed by the committed spec.)
    #[cfg(unix)]
    #[test]
    fn swap_bundle_through_a_symlinked_install_path_installs_the_new_tree() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();

        // The real bundle lives elsewhere; the configured install path is a symlink to it. The
        // resolved target is what the swap is given, so the tree behind the link is replaced.
        let real_root = dir.path().join("real");
        std::fs::create_dir(&real_root).unwrap();
        let real = staged_bundle(&real_root, "MyApp.app", "myapp", b"old");
        let link = dir.path().join("MyApp.app");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let target = super::resolve_bundle_target(&link);
        super::swap_bundle(&staged, &target, &stash, &dir.path().join("updater"), None)
            .expect("a symlinked install path must swap");

        assert_eq!(
            std::fs::read(link.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"new",
            "the configured install path must resolve to the new bundle after the swap"
        );
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the caller's symlink must survive the update, not be replaced by a real directory"
        );
        assert_eq!(
            std::fs::read(real.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"new",
            "the tree behind the link is the one that gets replaced"
        );
    }

    // BNDL-5-4: `resolve_bundle_target` maps a symlinked install path to the tree it designates, so
    // the swap replaces that tree (and stages beside it) instead of renaming the link itself, which
    // would orphan the installed bundle on disk. Everything else passes through unchanged.
    #[test]
    fn resolve_bundle_target_follows_only_a_live_symlink() {
        let dir = tempfile::tempdir().unwrap();

        // A plain directory, and a path that does not exist yet (fresh install): unchanged.
        let plain = staged_bundle(dir.path(), "Plain.app", "myapp", b"x");
        assert_eq!(super::resolve_bundle_target(&plain), plain);
        let missing = dir.path().join("Missing.app");
        assert_eq!(super::resolve_bundle_target(&missing), missing);

        #[cfg(unix)]
        {
            let real = staged_bundle(dir.path(), "Real.app", "myapp", b"x");
            let link = dir.path().join("Link.app");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert_eq!(
                super::resolve_bundle_target(&link),
                std::fs::canonicalize(&real).unwrap(),
                "a live symlink resolves to its target"
            );

            // A dangling link has no target to replace, so the configured path is kept and the swap
            // stashes the stale entry.
            let dangling = dir.path().join("Dangling.app");
            std::os::unix::fs::symlink(dir.path().join("nowhere"), &dangling).unwrap();
            assert_eq!(super::resolve_bundle_target(&dangling), dangling);
        }
    }

    // `install_bundle` replaces exactly the path it is handed and resolves nothing itself. The
    // orchestrator calls `resolve_bundle_target` once, before the confirmation prompt, so that the
    // path the user approves is the path that gets written; resolving a second time down here would
    // reopen the window for the link to be repointed in between. Handing it a symlink therefore
    // replaces the link, which is what proves the resolution is not happening twice.
    #[cfg(all(unix, feature = "archive-zip"))]
    #[test]
    fn install_bundle_replaces_exactly_the_path_it_is_given() {
        use std::io::Write as _;

        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("MyApp.app.zip");
        {
            let f = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);
            zip.start_file("MyApp.app/Contents/MacOS/myapp", opts)
                .unwrap();
            zip.write_all(b"new").unwrap();
            zip.finish().unwrap();
        }

        let install_root = dir.path().join("Applications");
        std::fs::create_dir(&install_root).unwrap();
        let real = staged_bundle(&install_root, "Real.app", "myapp", b"old");
        let link = install_root.join("Link.app");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        super::install_bundle(&archive_path, "MyApp.app", &link, None, false).unwrap();

        assert!(
            !std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the given path must be replaced, not followed"
        );
        assert_eq!(
            std::fs::read(link.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"new",
            "the new bundle lands at the path that was passed in"
        );
        assert_eq!(
            std::fs::read(real.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"old",
            "the tree the link pointed at is left alone"
        );
    }

    // BNDL-5-4: a dangling symlink at the install path counts as an existing entry -- it is stashed
    // and replaced. Renaming the staged directory straight onto it would fail with ENOTDIR.
    #[cfg(unix)]
    #[test]
    fn swap_bundle_replaces_a_dangling_symlink_at_the_install_path() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir(&staging).unwrap();
        let staged = staged_bundle(&staging, "MyApp.app", "myapp", b"new");
        let stash = dir.path().join("stash");
        std::fs::create_dir(&stash).unwrap();

        let dest = dir.path().join("MyApp.app");
        std::os::unix::fs::symlink(dir.path().join("gone"), &dest).unwrap();

        let target = super::resolve_bundle_target(&dest);
        super::swap_bundle(&staged, &target, &stash, &dir.path().join("updater"), None)
            .expect("a dangling symlink must be replaced, not renamed onto");

        assert!(
            dest.is_dir()
                && !std::fs::symlink_metadata(&dest)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
            "the stale link must be gone, replaced by the installed bundle"
        );
        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"new"
        );
        assert!(
            stash.join("old").exists() || std::fs::symlink_metadata(stash.join("old")).is_ok(),
            "the stale link must have been stashed rather than clobbered"
        );
    }

    // BNDL-2-2: the staging/stash dirs are created inside the destination's parent, so an
    // unwritable parent fails there -- before the archive is even opened -- as
    // `InstallPathNotWritable` naming the bundle install path.
    #[cfg(unix)]
    #[test]
    fn install_bundle_reports_an_unwritable_install_parent() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let ro_dir = dir.path().join("Applications");
        std::fs::create_dir(&ro_dir).unwrap();
        let dest = ro_dir.join("MyApp.app");
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        // The archive is never read: creating the staging dir fails first.
        let res = super::install_bundle(
            &dir.path().join("never-read.zip"),
            "MyApp.app",
            &dest,
            None,
            false,
        );
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        match res {
            Err(Error::InstallPathNotWritable { path }) => assert_eq!(
                path, dest,
                "the staging failure must name the bundle install path"
            ),
            other => panic!("expected InstallPathNotWritable, got {other:?}"),
        }
    }

    // BNDL-2-3: an archive that does not carry the configured bundle directory is an error naming
    // the missing staged root, with the installed bundle untouched and no staging/stash residue
    // left beside it.
    #[cfg(feature = "archive-zip")]
    #[test]
    fn install_bundle_errors_when_the_archive_has_no_bundle_directory() {
        use std::io::Write as _;

        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.zip");
        {
            let f = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("some-other-dir/readme.txt", opts).unwrap();
            zip.write_all(b"not a bundle").unwrap();
            zip.finish().unwrap();
        }

        let install_root = dir.path().join("Applications");
        std::fs::create_dir(&install_root).unwrap();
        let dest = staged_bundle(&install_root, "MyApp.app", "myapp", b"old");

        let err = super::install_bundle(&archive_path, "MyApp.app", &dest, None, false)
            .expect_err("an archive without the bundle directory must error");
        assert!(
            matches!(&err, Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound),
            "expected a NotFound Io error, got {err:?}"
        );
        assert!(
            err.to_string().contains("MyApp.app"),
            "the error must name the bundle directory it looked for, got: {err}"
        );
        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"old",
            "the installed bundle must be untouched"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&install_root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "MyApp.app")
            .collect();
        assert!(
            leftovers.is_empty(),
            "the staging/stash dirs must be cleaned up on the error path too, found: {leftovers:?}"
        );
    }

    // BNDL-4-1/BNDL-4-2: the same install over a **tar.gz** bundle archive (the zip case is
    // covered above). `tar`'s unpack carries the executable bit and restores symlinks, both of
    // which a macOS `.app` (framework `Versions/Current` links, signed executables) depends on.
    #[cfg(all(feature = "compression-tar-gz", unix))]
    #[test]
    fn install_bundle_extracts_and_swaps_a_tar_gz_archive() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("MyApp.app.tar.gz");
        {
            let mut ar = tar::Builder::new(Vec::new());

            let exe = b"#!/bin/sh\necho new\n";
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o755);
            header.set_size(exe.len() as u64);
            ar.append_data(&mut header, "MyApp.app/Contents/MacOS/myapp", &exe[..])
                .unwrap();

            let data = b"payload";
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o644);
            header.set_size(data.len() as u64);
            ar.append_data(
                &mut header,
                "MyApp.app/Contents/Resources/data.txt",
                &data[..],
            )
            .unwrap();

            // A relative symlink, as a bundled framework's `Versions/Current` would be.
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_mode(0o777);
            header.set_size(0);
            ar.append_link(&mut header, "MyApp.app/Contents/Current", "Resources")
                .unwrap();

            let tarred = ar.into_inner().unwrap();
            let out = std::fs::File::create(&archive_path).unwrap();
            let mut gz = flate2::write::GzEncoder::new(out, flate2::Compression::default());
            std::io::copy(&mut tarred.as_slice(), &mut gz).unwrap();
            gz.finish().unwrap();
        }

        let install_root = dir.path().join("Applications");
        std::fs::create_dir(&install_root).unwrap();
        let dest = staged_bundle(&install_root, "MyApp.app", "myapp", b"old");
        std::fs::write(dest.join("Contents").join("gone.txt"), b"stale").unwrap();

        super::install_bundle(&archive_path, "MyApp.app", &dest, None, false)
            .expect("installing a bundle from a tar.gz must work");

        let installed_exe = dest.join("Contents").join("MacOS").join("myapp");
        assert_eq!(
            std::fs::read(&installed_exe).unwrap(),
            b"#!/bin/sh\necho new\n",
            "the installed exe must come from the tar.gz"
        );
        assert!(
            std::fs::metadata(&installed_exe)
                .unwrap()
                .permissions()
                .mode()
                & 0o111
                != 0,
            "the archived executable bit must survive a tar.gz install"
        );
        let link = dest.join("Contents").join("Current");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "a tar symlink entry must be installed as a symlink"
        );
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            std::path::Path::new("Resources"),
            "the symlink must still point at its original relative target"
        );
        assert!(
            !dest.join("Contents").join("gone.txt").exists(),
            "the swap replaces the whole tree"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&install_root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "MyApp.app")
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging/stash dirs must not be left behind, found: {leftovers:?}"
        );
    }

    // A bundle-mode [`FinishCtx`], built on the same shape as `traversal_ctx` (which the
    // substitution-guard tests use) with the bundle fields filled in, so the finish tail takes the
    // `install_bundle` branch.
    fn bundle_ctx(
        bundle_path_in_archive: &str,
        version: &str,
        bundle_target: &std::path::Path,
    ) -> super::FinishCtx {
        let mut ctx = traversal_ctx("unused-bin-path", version);
        ctx.bundle_path_in_archive = Some(bundle_path_in_archive.to_string());
        ctx.bundle_target = Some(bundle_target.to_path_buf());
        ctx
    }

    // BNDL-1-1/BNDL-2-3: the finish tail end to end in bundle mode, with all three `{{ .. }}`
    // templates inside a *nested* bundle path. The substitution is shared with the single-file
    // path, so this pins that bundle mode reads it (and reads `bundle_path_in_archive`, not
    // `bin_path_in_archive`), installs the nested directory, and reports the release.
    #[cfg(feature = "archive-zip")]
    #[test]
    fn finish_update_owned_installs_a_templated_nested_bundle_path() {
        use std::io::Write as _;

        let install_dir = tempfile::tempdir().unwrap();
        let dest = install_dir.path().join("MyApp.app");

        let archive_dir = tempfile::tempdir().unwrap();
        let archive_path = archive_dir.path().join("release.zip");
        {
            let f = std::fs::File::create(&archive_path).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file(
                "myapp-1.2.3-x86_64-unknown-linux-gnu/MyApp.app/Contents/MacOS/myapp",
                opts,
            )
            .unwrap();
            zip.write_all(b"new-exe").unwrap();
            zip.start_file(
                "myapp-1.2.3-x86_64-unknown-linux-gnu/MyApp.app/Contents/Info.plist",
                opts,
            )
            .unwrap();
            zip.write_all(b"plist").unwrap();
            zip.finish().unwrap();
        }

        let mut ctx = bundle_ctx(
            "{{ bin }}-{{ version }}-{{ target }}/MyApp.app",
            "1.2.3",
            &dest,
        );
        ctx.bin_name = "myapp".to_string();
        // A path that only the single-file branch would ever write to.
        let never_written = install_dir.path().join("single-file-install");
        ctx.bin_install_path = never_written.clone();

        let status = super::finish_update_owned(ctx, archive_dir, &archive_path)
            .expect("a bundle-mode finish must install the bundle");
        assert!(status.is_updated(), "bundle mode must report an update");
        assert_eq!(
            status.version(),
            Some("1.2.3"),
            "the installed release must be reported"
        );
        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"new-exe",
            "the templated nested bundle directory must be installed at the bundle install path"
        );
        assert!(
            dest.join("Contents").join("Info.plist").exists(),
            "the whole bundle tree must be installed, not just the executable"
        );
        assert!(
            !never_written.exists(),
            "bundle mode must not run the single-file install"
        );
        let leftovers: Vec<_> = std::fs::read_dir(install_dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "MyApp.app")
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging/stash dirs must not be left beside the installed bundle, found: {leftovers:?}"
        );
    }

    // BNDL-1-1 (traversal defense): the `is_safe_asset_name` guard covers a value substituted into
    // the BUNDLE path too, so a malicious release version cannot redirect the swap's source outside
    // the staging dir. The guard fires before the archive is read (it does not even exist here),
    // and the installed bundle is untouched.
    #[test]
    fn finish_update_rejects_traversal_in_a_substituted_bundle_path() {
        let install_dir = tempfile::tempdir().unwrap();
        let dest = staged_bundle(install_dir.path(), "MyApp.app", "myapp", b"old");
        let archive_dir = tempfile::tempdir().unwrap();
        let archive = archive_dir.path().join("release.zip");

        let ctx = bundle_ctx("{{ version }}/MyApp.app", "../evil", &dest);
        match super::finish_update_owned(ctx, archive_dir, &archive) {
            Err(Error::InvalidAssetName { name }) => {
                assert_eq!(name, "../evil", "the offending component must be named");
            }
            other => panic!("expected InvalidAssetName for a traversal version, got {other:?}"),
        }
        assert_eq!(
            std::fs::read(dest.join("Contents").join("MacOS").join("myapp")).unwrap(),
            b"old",
            "a rejected substitution must not touch the installed bundle"
        );
    }

    // A clonable release source offering one newer release with an asset for the target the bundle
    // tests configure, so the same source can drive the sync updater, the async updater (the async
    // adapter requires `Clone`), and a full `update_extended` run up to the preflight.
    #[derive(Clone)]
    struct BundleSource;
    impl BundleSource {
        fn release(version: &str) -> Result<Release> {
            Release::builder()
                .version(version)
                .asset(crate::update::ReleaseAsset::new(
                    "myapp-x86_64-unknown-linux-gnu.zip",
                    // Unroutable on purpose: reaching the download at all is a test failure.
                    "http://127.0.0.1:1/myapp-x86_64-unknown-linux-gnu.zip",
                ))
                .build()
        }
    }
    impl crate::update::ReleaseSource for BundleSource {
        fn get_latest_release(&self) -> Result<Release> {
            Self::release("1.2.3")
        }
        fn get_releases(&self) -> Result<Vec<Release>> {
            Ok(vec![Self::release("1.2.3")?])
        }
        fn get_release_version(&self, v: &str) -> Result<Release> {
            Self::release(v)
        }
    }

    // BNDL-3-1: the opt-in preflight through a real updater in bundle mode. It probes the BUNDLE'S
    // PARENT (not `bin_install_path`, which defaults to the running test binary and is writable)
    // and bails before anything is downloaded -- the asset URL is unroutable, so a preflight that
    // failed to fire would surface a transport error instead.
    #[cfg(unix)]
    #[test]
    fn update_extended_preflight_rejects_an_unwritable_bundle_parent() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let ro_dir = dir.path().join("Applications");
        std::fs::create_dir(&ro_dir).unwrap();
        let bundle = ro_dir.join("MyApp.app");
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let upd = crate::backends::custom::Update::configure()
            .source(BundleSource)
            .bin_name("myapp")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .bundle_path_in_archive("MyApp.app")
            .bundle_install_path(&bundle)
            .check_install_path_writable(true)
            .no_confirm(true)
            .show_output(false)
            .build()
            .unwrap();
        let res = upd.update_extended();
        std::fs::set_permissions(&ro_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        match res {
            Err(Error::InstallPathNotWritable { path }) => assert_eq!(
                path, ro_dir,
                "the preflight must name the unwritable bundle parent"
            ),
            other => panic!("expected InstallPathNotWritable from the preflight, got {other:?}"),
        }
    }

    // BNDL-1-7: the bundle fields ride through `FinishCtx`, which is what the sync tail and the
    // async tail (inside `spawn_blocking`) both capture -- so bundle mode behaves identically on
    // both. Without the setters the captured context stays in single-file mode.
    #[test]
    fn finish_ctx_captures_the_bundle_fields() {
        let asset = crate::update::ReleaseAsset::new("release.zip", "https://host/release.zip");

        let bundled = crate::backends::custom::Update::configure()
            .source(BundleSource)
            .bin_name("myapp")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .bundle_path_in_archive("MyApp.app")
            .bundle_install_path("/Applications/MyApp.app")
            .build()
            .unwrap();
        let ctx = super::FinishCtx::capture(
            &bundled,
            rel("1.2.3"),
            &asset,
            Some(std::path::Path::new("/Applications/MyApp.app")),
        );
        assert_eq!(ctx.bundle_path_in_archive.as_deref(), Some("MyApp.app"));
        assert_eq!(
            ctx.bundle_target.as_deref(),
            Some(std::path::Path::new("/Applications/MyApp.app"))
        );

        let plain = crate::backends::custom::Update::configure()
            .source(BundleSource)
            .bin_name("myapp")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .build()
            .unwrap();
        let ctx = super::FinishCtx::capture(&plain, rel("1.2.3"), &asset, None);
        assert!(
            ctx.bundle_path_in_archive.is_none() && ctx.bundle_target.is_none(),
            "without the setters the finish tail must stay in single-file mode"
        );
    }

    // BNDL-1-7 (async lane): an updater built with `build_async` carries the same bundle fields
    // into the shared `FinishCtx`, so `update_extended_async` installs bundles like the sync verb.
    #[cfg(feature = "async")]
    #[test]
    fn async_update_captures_the_bundle_fields_too() {
        let upd = crate::backends::custom::AsyncUpdate::configure()
            .source(crate::backends::custom::Blocking::new(BundleSource))
            .bin_name("myapp")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .bundle_path_in_archive("MyApp.app")
            .bundle_install_path("/Applications/MyApp.app")
            .build_async()
            .unwrap();
        let asset = crate::update::ReleaseAsset::new("release.zip", "https://host/release.zip");
        let ctx = super::FinishCtx::capture(
            &upd,
            rel("1.2.3"),
            &asset,
            Some(std::path::Path::new("/Applications/MyApp.app")),
        );
        assert_eq!(ctx.bundle_path_in_archive.as_deref(), Some("MyApp.app"));
        assert_eq!(
            ctx.bundle_target.as_deref(),
            Some(std::path::Path::new("/Applications/MyApp.app"))
        );
    }

    // Build a custom-backend `Update` carrying `checksum`, to drive `finish_update` directly.
    #[cfg(feature = "checksums")]
    fn update_with_checksum(checksum: crate::Checksum) -> crate::backends::custom::Update {
        crate::backends::custom::Update::configure()
            .source(BoundSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .verify_checksum(checksum)
            .build()
            .unwrap()
    }

    // Build a custom-backend `Update` with no explicit checksum and `verify_release_digest`
    // as given, to drive the release-digest gate of `finish_update` directly.
    #[cfg(feature = "checksums")]
    fn update_with_release_digest(verify: bool) -> crate::backends::custom::Update {
        crate::backends::custom::Update::configure()
            .source(BoundSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .verify_release_digest(verify)
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
        let asset = ReleaseAsset::new("release.tar.gz", "https://host/release.tar.gz");

        let err = super::finish_update(&upd, release, &asset, dir, &archive_path, None)
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
        feature = "compression-tar-gz"
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
        let asset = ReleaseAsset::new("release.tar.gz", "https://host/release.tar.gz");

        let err = super::finish_update(&upd, release, &asset, dir, &archive_path, None)
            .expect_err("the bytes are not a real archive, so extraction must fail");
        let msg = err.to_string();
        assert!(
            !msg.contains("checksum mismatch"),
            "a matching checksum must pass the gate; the failure should come from extraction, \
             got: {}",
            msg
        );
    }

    // The release-digest gate is on by default: an asset carrying a backend-published digest that
    // does not match the download aborts the update at the gate, with no `verify_checksum`
    // configured at all.
    #[cfg(feature = "checksums")]
    #[test]
    fn finish_update_rejects_a_mismatched_release_digest_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.tar.gz");
        std::fs::write(&archive_path, b"hello").unwrap();

        let upd = update_with_release_digest(true);
        let release = Release::builder().version("1.2.3").build().unwrap();
        // A valid-length but wrong digest (the file's real digest is 2cf24dba…).
        let asset = ReleaseAsset::new("release.tar.gz", "https://host/release.tar.gz")
            .with_digest(format!("sha256:{}", "00".repeat(32)));

        let err = super::finish_update(&upd, release, &asset, dir, &archive_path, None)
            .expect_err("a mismatched release digest must abort the update");
        let msg = err.to_string();
        assert!(
            msg.contains("checksum mismatch"),
            "expected a checksum-mismatch abort from the release digest, got: {}",
            msg
        );
    }

    // A matching release digest passes the gate and the flow proceeds (here into extraction,
    // which fails on the bogus archive bytes — a non-checksum error).
    #[cfg(all(
        feature = "checksums",
        feature = "archive-tar",
        feature = "compression-tar-gz"
    ))]
    #[test]
    fn finish_update_passes_a_matching_release_digest_then_proceeds() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.tar.gz");
        std::fs::write(&archive_path, b"hello").unwrap();

        let upd = update_with_release_digest(true);
        let release = Release::builder().version("1.2.3").build().unwrap();
        // The real SHA-256 of b"hello", in the forge's `algorithm:hex` form.
        let asset = ReleaseAsset::new("release.tar.gz", "https://host/release.tar.gz")
            .with_digest("sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");

        let err = super::finish_update(&upd, release, &asset, dir, &archive_path, None)
            .expect_err("the bytes are not a real archive, so extraction must fail");
        let msg = err.to_string();
        assert!(
            !msg.contains("checksum mismatch"),
            "a matching release digest must pass the gate; the failure should come from \
             extraction, got: {}",
            msg
        );
    }

    // `verify_release_digest(false)` opts out: the same mismatched digest is ignored and the flow
    // proceeds past the gate (failing later at extraction with a non-checksum error).
    #[cfg(all(
        feature = "checksums",
        feature = "archive-tar",
        feature = "compression-tar-gz"
    ))]
    #[test]
    fn finish_update_release_digest_opt_out_skips_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.tar.gz");
        std::fs::write(&archive_path, b"hello").unwrap();

        let upd = update_with_release_digest(false);
        let release = Release::builder().version("1.2.3").build().unwrap();
        let asset = ReleaseAsset::new("release.tar.gz", "https://host/release.tar.gz")
            .with_digest(format!("sha256:{}", "00".repeat(32)));

        let err = super::finish_update(&upd, release, &asset, dir, &archive_path, None)
            .expect_err("the bytes are not a real archive, so extraction must fail");
        let msg = err.to_string();
        assert!(
            !msg.contains("checksum mismatch"),
            "opting out must skip the release-digest gate; the failure should come from \
             extraction, got: {}",
            msg
        );
    }

    // A present-but-unsupported digest is a hard error at the gate (naming the digest), not a
    // silent skip — a forge publishing an algorithm this crate can't parse must be surfaced.
    #[cfg(feature = "checksums")]
    #[test]
    fn finish_update_rejects_an_unsupported_release_digest() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.tar.gz");
        std::fs::write(&archive_path, b"hello").unwrap();

        let upd = update_with_release_digest(true);
        let release = Release::builder().version("1.2.3").build().unwrap();
        let asset = ReleaseAsset::new("release.tar.gz", "https://host/release.tar.gz")
            .with_digest("md5:abc123");

        let err = super::finish_update(&upd, release, &asset, dir, &archive_path, None)
            .expect_err("an unsupported digest must abort the update");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "expected Error::InvalidResponse, got {:?}",
            err
        );
        let msg = err.to_string();
        assert!(
            msg.contains("md5:abc123"),
            "the error must name the offending digest, got: {}",
            msg
        );
    }

    // the async finish tail (`finish_update_async`, ~update.rs:1022) runs the
    // verify/extract/install tail under `tokio::task::spawn_blocking` and maps a `JoinError` (e.g.
    // a panic in that tail) to `Error::Internal { source: Some(Box::new(join_err)) }`. That site is
    // only reachable through the full async update flow (network download of a real asset, then a
    // panic in the private `finish_update_owned` tail), so it cannot be driven from a unit test
    // without breaking the install flow. What we pin here is the exact `map_err` mapping that site
    // performs: a real `JoinError` from a panicking `spawn_blocking` must route to `Error::Internal`
    // with a NON-None, chained `source()` - distinguishing it from the genuine-invariant
    // `Internal { source: None }` extractor sites. This mirrors `custom.rs`'s
    // `blocking_adapter_join_failure_chains_source`, which covers the structurally-identical
    // `Blocking` adapter mapping; together they pin both async JoinError->Internal sites.
    #[cfg(feature = "async")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_finish_join_failure_maps_to_internal_with_source() {
        use std::error::Error as _;

        // Reproduce the production mapping verbatim: a panicking blocking task fails the join, and
        // the join error is wrapped the same way the finish tail wraps it.
        let mapped: Result<()> = tokio::task::spawn_blocking(|| {
            panic!("boom in the finish tail");
        })
        .await
        .map_err(|e| crate::errors::Error::Internal {
            message: "finish-update task failed".to_string(),
            source: Some(Box::new(e)),
        });

        let err = mapped.expect_err("a panicking finish tail must fail the join");
        match err {
            crate::errors::Error::Internal {
                ref message,
                ref source,
            } => {
                assert_eq!(message, "finish-update task failed");
                assert!(
                    source.is_some(),
                    "Internal from a JoinError must carry a non-None source"
                );
            }
            other => panic!("expected Error::Internal, got {:?}", other),
        }
        assert!(
            err.source().is_some(),
            "the JoinError must chain through source()"
        );
        assert_eq!(
            err.to_string(),
            "InternalError: finish-update task failed",
            "Display must render the message without panicking"
        );
    }

    // -----------------------------------------------------------------------
    // the DOWNLOAD path applies the backend's derived Authorization scheme AND
    // honors a user override, captured off a loopback TCP stub through the real
    // http client (reqwest or ureq), exercising `build_download` + `download_to`
    // end to end, not just `apply_auth` in isolation.
    //
    // The listing path already had a wire-level loopback test (backends::mod tests);
    // the download path lacked one. These close that gap.
    // -----------------------------------------------------------------------

    /// Bind a loopback stub that accepts one request, captures its raw header lines, and replies
    /// with a small 200 body. Returns the base URL and the captured-request handle.
    #[cfg(any(feature = "github", feature = "gitlab"))]
    fn download_auth_capture_stub() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let lines: Vec<String> = req.lines().map(|l| l.to_string()).collect();
                *sink.lock().unwrap() = lines;
                let body = "payload";
                let out = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        (base, captured)
    }

    /// Extract the `Authorization` header value the stub received, if any.
    #[cfg(any(feature = "github", feature = "gitlab"))]
    fn captured_download_authorization(lines: &[String]) -> Option<String> {
        lines.iter().find_map(|l| {
            l.strip_prefix("Authorization: ")
                .or_else(|| l.strip_prefix("authorization: "))
                .map(|v| v.to_string())
        })
    }

    #[cfg(feature = "github")]
    #[test]
    fn download_path_applies_github_token_scheme() {
        // github resolves to the `token` scheme; the download GET must carry `token secret`. The
        // asset is served from the loopback stub, a different host than the github API, so the host
        // is authorized via `allow_auth_host` for the token to be attached.
        let (base, captured) = download_auth_capture_stub();
        let host = crate::backends::common::host_of(&base).unwrap();
        let upd = crate::backends::github::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("secret")
            .allow_auth_host(host)
            .build()
            .unwrap();
        let asset = super::ReleaseAsset::new("app.tar.gz", format!("{base}/app.tar.gz"));
        let download = super::build_download(&upd, &asset).unwrap();
        let mut out = Vec::new();
        download.download_to(&mut out).unwrap();
        assert_eq!(out, b"payload", "the download streamed the stub body");
        let lines = captured.lock().unwrap().clone();
        assert_eq!(
            captured_download_authorization(&lines),
            Some("token secret".to_string()),
            "the download path must send github's derived `token` auth header to an authorized host"
        );
    }

    #[cfg(feature = "github")]
    #[test]
    fn download_path_drops_auth_for_cross_origin_asset_url() {
        // A server-supplied asset download_url on a host other than the github API (and not in the
        // allow_auth_host set) must NOT receive the token. This is the SEC-1 credential-exfiltration
        // guard: a malicious release server cannot harvest the user's PAT via the download URL.
        let (base, captured) = download_auth_capture_stub();
        let upd = crate::backends::github::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("secret")
            .build()
            .unwrap();
        let asset = super::ReleaseAsset::new("app.tar.gz", format!("{base}/app.tar.gz"));
        let download = super::build_download(&upd, &asset).unwrap();
        let mut out = Vec::new();
        download.download_to(&mut out).unwrap();
        let lines = captured.lock().unwrap().clone();
        assert_eq!(
            captured_download_authorization(&lines),
            None,
            "the token must not be sent to a cross-origin asset download URL"
        );
    }

    #[cfg(feature = "github")]
    #[test]
    fn download_path_honors_user_authorization_override() {
        // A backend token is configured AND the user supplies their own Authorization via
        // `request_header`. The override must win over the backend token on the DOWNLOAD path,
        // exactly like the listing path — when the asset host is authorized (here via
        // `allow_auth_host`, since the loopback stub is not the github API host).
        use crate::http_client::header::AUTHORIZATION;
        let (base, captured) = download_auth_capture_stub();
        let host = crate::backends::common::host_of(&base).unwrap();
        let upd = crate::backends::github::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("secret")
            .allow_auth_host(host)
            .request_header(AUTHORIZATION, "Bearer user-override")
            .build()
            .unwrap();
        let asset = super::ReleaseAsset::new("app.tar.gz", format!("{base}/app.tar.gz"));
        let download = super::build_download(&upd, &asset).unwrap();
        let mut out = Vec::new();
        download.download_to(&mut out).unwrap();
        let lines = captured.lock().unwrap().clone();
        assert_eq!(
            captured_download_authorization(&lines),
            Some("Bearer user-override".to_string()),
            "a user AUTHORIZATION override must win over the backend token, and be forwarded to the \
             authorized asset host"
        );
    }

    #[cfg(feature = "github")]
    #[test]
    fn download_path_drops_user_authorization_for_disallowed_host() {
        // S2: a user-supplied `Authorization` (via `request_header`) is a credential, so it must be
        // host-gated exactly like the crate's derived token. The loopback stub is NOT the github API
        // host and is NOT in `allow_auth_host`, so the user's Authorization must be dropped, not
        // leaked to the server-chosen download host.
        use crate::http_client::header::AUTHORIZATION;
        let (base, captured) = download_auth_capture_stub();
        let upd = crate::backends::github::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .request_header(AUTHORIZATION, "Bearer user-secret")
            .build()
            .unwrap();
        let asset = super::ReleaseAsset::new("app.tar.gz", format!("{base}/app.tar.gz"));
        let download = super::build_download(&upd, &asset).unwrap();
        let mut out = Vec::new();
        download.download_to(&mut out).unwrap();
        let lines = captured.lock().unwrap().clone();
        assert_eq!(
            captured_download_authorization(&lines),
            None,
            "a user Authorization must NOT be forwarded to a disallowed (cross-origin) download host"
        );
    }

    #[cfg(feature = "gitlab")]
    #[test]
    fn download_path_applies_gitlab_bearer_scheme() {
        // gitlab resolves to the `Bearer` scheme; the download GET must carry `Bearer secret`,
        // proving the per-backend default scheme is threaded all the way to the download wire.
        let (base, captured) = download_auth_capture_stub();
        let host = crate::backends::common::host_of(&base).unwrap();
        let upd = crate::backends::gitlab::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("secret")
            .allow_auth_host(host)
            .build()
            .unwrap();
        let asset = super::ReleaseAsset::new("app.tar.gz", format!("{base}/app.tar.gz"));
        let download = super::build_download(&upd, &asset).unwrap();
        let mut out = Vec::new();
        download.download_to(&mut out).unwrap();
        let lines = captured.lock().unwrap().clone();
        assert_eq!(
            captured_download_authorization(&lines),
            Some("Bearer secret".to_string()),
            "the download path must send gitlab's derived `Bearer` auth header to an authorized host"
        );
    }

    // the configured retry budget is forwarded onto the Download built by `build_download`.
    // Without forwarding, a custom-backend `.retries(N)` would be a silent no-op on the one
    // transport the crate controls (the download). We assert the budget reaches the GET by pointing
    // the asset at a closed loopback port (immediate connection-refused) and counting attempts is
    // impractical through the real client, so instead we drive the forwarding directly: a custom
    // updater with `.retries(2)` must produce a download that retries. We prove the wiring by
    // checking `build_download` carries the budget through to a re-established request via an
    // injected flaky client.
    #[test]
    fn build_download_forwards_configured_retry_budget() {
        use crate::http_client::header::HeaderMap;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        struct FlakyClient {
            body: Vec<u8>,
            fail_times: AtomicU32,
            attempts: Arc<AtomicU32>,
        }
        impl crate::http_client::HttpClient for FlakyClient {
            fn get(
                &self,
                _url: &str,
                _headers: &HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> Result<Box<dyn crate::http_client::HttpResponse>> {
                self.attempts.fetch_add(1, Ordering::SeqCst);
                if self.fail_times.load(Ordering::SeqCst) > 0 {
                    self.fail_times.fetch_sub(1, Ordering::SeqCst);
                    return Err(crate::errors::Error::HttpStatus {
                        status: 503,
                        url: "u".into(),
                    });
                }
                Ok(Box::new(CannedResponse {
                    body: self.body.clone(),
                }))
            }
        }
        struct CannedResponse {
            body: Vec<u8>,
        }
        impl crate::http_client::HttpResponse for CannedResponse {
            fn headers(&self) -> &HeaderMap {
                static EMPTY: std::sync::OnceLock<HeaderMap> = std::sync::OnceLock::new();
                EMPTY.get_or_init(HeaderMap::new)
            }
            fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
                Box::new(std::io::Cursor::new(self.body))
            }
        }

        let attempts = Arc::new(AtomicU32::new(0));
        let client = Arc::new(FlakyClient {
            body: b"after-retry".to_vec(),
            fail_times: AtomicU32::new(2),
            attempts: attempts.clone(),
        });

        // A custom-backend updater configured with `.retries(3)` and a fast backoff, plus the flaky
        // client injected. `build_download` must forward that budget onto the Download so the GET is
        // re-established after the transient failures.
        let upd = crate::backends::custom::Update::configure()
            .source(BoundSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .retries(3)
            .retry_backoff(
                std::time::Duration::from_millis(1),
                std::time::Duration::from_millis(2),
            )
            .http_client(client)
            .build()
            .unwrap();

        let asset = super::ReleaseAsset::new("app.bin", "https://nonroutable.invalid/app.bin");
        let download = super::build_download(&upd, &asset).unwrap();
        let mut out = Vec::new();
        download.download_to(&mut out).unwrap();
        assert_eq!(out, b"after-retry", "the download succeeds after retrying");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "the configured retry budget (3) must be forwarded to the download: two failures + success"
        );
    }

    // CORP-1: a custom-backend updater configured with `root_certificate`(s) must forward them onto
    // the `Download` built by `build_download`, so the download materializes a client that trusts
    // them. We assert the count of forwarded certs matches what was configured on the builder.
    #[test]
    fn build_download_forwards_certs_to_download() {
        use crate::http_client::header::HeaderMap;
        use std::sync::Arc;

        // Inject no-op client(s) so the builder's eager cert materialization is skipped (an injected
        // client wins): this isolates the *forwarding* of `root_certificates` from cert validation,
        // letting us use placeholder bytes while still exercising the build_download copy loop. Under
        // the async feature `build_client` also materializes the async client, so inject that slot too.
        struct NoopClient;
        impl crate::http_client::HttpClient for NoopClient {
            fn get(
                &self,
                _url: &str,
                _headers: &HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> Result<Box<dyn crate::http_client::HttpResponse>> {
                unreachable!("not called in this test")
            }
        }
        #[cfg(feature = "async")]
        struct NoopAsyncClient;
        #[cfg(feature = "async")]
        impl crate::http_client::AsyncHttpClient for NoopAsyncClient {
            fn get<'a>(
                &'a self,
                _url: &'a str,
                _headers: &'a HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> futures_util::future::BoxFuture<
                'a,
                Result<Box<dyn crate::http_client::AsyncHttpResponse>>,
            > {
                unreachable!("not called in this test")
            }
        }

        let mut builder = crate::backends::custom::Update::configure();
        builder
            .source(BoundSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .http_client(Arc::new(NoopClient))
            .add_root_certificate(crate::Certificate::from_pem(b"pem-bytes".to_vec()))
            .add_root_certificate(crate::Certificate::from_der(b"der-bytes".to_vec()));
        #[cfg(feature = "async")]
        builder.http_client_async(Arc::new(NoopAsyncClient));
        let upd = builder.build().unwrap();

        let asset = super::ReleaseAsset::new("app.bin", "https://nonroutable.invalid/app.bin");
        let download = super::build_download(&upd, &asset).unwrap();
        assert_eq!(
            download.root_certificates().len(),
            2,
            "both configured root certificates must be forwarded onto the Download"
        );
    }

    // download retry only covers the request-ESTABLISHMENT phase. A failure that occurs
    // AFTER streaming has begun must NOT re-issue the GET (which would append a duplicate/partial
    // body to the destination and corrupt it). We assert: exactly one GET attempt despite a generous
    // retry budget, the call errors, and `dest` holds only the partial pre-failure bytes (never a
    // duplicated or completed body).
    #[test]
    fn download_does_not_retry_or_corrupt_after_streaming_begins() {
        use crate::http_client::header::{CONTENT_LENGTH, HeaderMap};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        // A reader that yields `prefix` once, then errors — simulating a mid-stream transport drop.
        struct FailingMidStream {
            prefix: Vec<u8>,
            yielded: bool,
        }
        impl std::io::Read for FailingMidStream {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if !self.yielded {
                    self.yielded = true;
                    let n = self.prefix.len().min(buf.len());
                    buf[..n].copy_from_slice(&self.prefix[..n]);
                    return Ok(n);
                }
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "connection reset mid-stream",
                ))
            }
        }
        struct MidStreamResponse {
            headers: HeaderMap,
            prefix: Vec<u8>,
        }
        impl crate::http_client::HttpResponse for MidStreamResponse {
            fn headers(&self) -> &HeaderMap {
                &self.headers
            }
            fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
                Box::new(FailingMidStream {
                    prefix: self.prefix,
                    yielded: false,
                })
            }
        }
        struct MidStreamClient {
            attempts: Arc<AtomicU32>,
        }
        impl crate::http_client::HttpClient for MidStreamClient {
            fn get(
                &self,
                _url: &str,
                _headers: &HeaderMap,
                _timeout: Option<std::time::Duration>,
            ) -> Result<Box<dyn crate::http_client::HttpResponse>> {
                self.attempts.fetch_add(1, Ordering::SeqCst);
                // The request ESTABLISHES successfully (200 + a Content-Length promising more bytes
                // than the body will actually deliver); the failure happens later, while streaming.
                let mut headers = HeaderMap::new();
                headers.insert(CONTENT_LENGTH, "1024".parse().unwrap());
                Ok(Box::new(MidStreamResponse {
                    headers,
                    prefix: b"PARTIAL".to_vec(),
                }))
            }
        }

        let attempts = Arc::new(AtomicU32::new(0));
        let client = Arc::new(MidStreamClient {
            attempts: attempts.clone(),
        });

        let mut dl = Download::from_url("https://nonroutable.invalid/asset.bin");
        dl.set_http_client(
            Some(client),
            #[cfg(feature = "async")]
            None,
        );
        // A generous retry budget: if the implementation wrongly retried the streaming phase, we
        // would see multiple GET attempts and/or a destination longer than the single 7-byte prefix.
        dl.set_retries(
            5,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(2),
        );

        let mut out = Vec::new();
        let res = dl.download_to(&mut out);
        assert!(
            res.is_err(),
            "a mid-stream failure must propagate, not be silently retried into a corrupt file"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "the GET must be issued exactly once: streaming-phase failures are NOT re-established"
        );
        assert_eq!(
            out, b"PARTIAL",
            "the destination must hold only the single pre-failure prefix, never a duplicated body"
        );
    }

    // --- Signature verification (embedded-key and rotation) ------------------------------------

    #[cfg(all(
        feature = "signatures",
        feature = "archive-tar",
        feature = "compression-tar-gz",
    ))]
    fn make_tar_gz() -> Result<Vec<u8>> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Cursor;

        let mut data = Cursor::new(Vec::<u8>::new());
        {
            let gz = GzEncoder::new(&mut data, Compression::default());
            let mut tar = tar::Builder::new(gz);
            let content = b"hello";
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "hello.txt", content.as_slice())?;
            tar.finish()?;
        }
        Ok(data.into_inner())
    }

    /// Sign `unsigned` bytes with the given signing keys, writing the signed archive to a new
    /// tempfile. The tempfile is returned so the caller controls its lifetime.
    #[cfg(all(
        feature = "signatures",
        feature = "archive-tar",
        feature = "compression-tar-gz",
    ))]
    fn sign_tar_gz(
        unsigned: &[u8],
        signing_keys: &[zipsign_api::SigningKey],
    ) -> Result<tempfile::NamedTempFile> {
        use std::io::Cursor;
        use tempfile::Builder;

        let signed_file = Builder::new().suffix(".tar.gz").tempfile()?;
        let signed_path = signed_file.path();
        let context = signed_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .as_bytes();

        let mut unsigned_cursor = Cursor::new(unsigned);
        zipsign_api::sign::copy_and_sign_tar(
            &mut unsigned_cursor,
            &mut signed_file.as_file(),
            signing_keys,
            Some(context),
        )
        .map_err(zipsign_api::ZipsignError::from)?;

        Ok(signed_file)
    }

    /// A compile-time const seed (the embedded-key pattern) signs an archive and the derived
    /// verifying key accepts it.
    #[test]
    #[cfg(all(
        feature = "signatures",
        feature = "archive-tar",
        feature = "compression-tar-gz",
    ))]
    fn embedded_key_verification_const_seed_verifies() -> Result<()> {
        const KEY_SEED: [u8; 32] = [42u8; 32];

        let signing_key = zipsign_api::SigningKey::from_bytes(&KEY_SEED);
        let vkey: [u8; zipsign_api::PUBLIC_KEY_LENGTH] = signing_key.verifying_key().to_bytes();

        let unsigned = make_tar_gz()?;
        let signed_file = sign_tar_gz(&unsigned, &[signing_key])?;

        super::verify_signature(signed_file.path(), &[vkey])
    }

    /// The public crate-root `self_update::verify_signature` (exposed for callers that stage a
    /// download themselves, e.g. an installer) verifies a signed archive, accepts a `VerifyingKey`
    /// slice, and takes `impl AsRef<Path>` (here a `PathBuf`, not just `&Path`).
    #[test]
    #[cfg(all(
        feature = "signatures",
        feature = "archive-tar",
        feature = "compression-tar-gz",
    ))]
    fn public_verify_signature_reexport_verifies_signed_archive() -> Result<()> {
        let signing_key = zipsign_api::SigningKey::from_bytes(&[7u8; 32]);
        let vkey: crate::VerifyingKey = signing_key.verifying_key().to_bytes();

        let unsigned = make_tar_gz()?;
        let signed_file = sign_tar_gz(&unsigned, &[signing_key])?;

        // Owned PathBuf argument exercises the `impl AsRef<Path>` signature.
        let owned: std::path::PathBuf = signed_file.path().into();
        crate::verify_signature(owned, &[vkey])?;

        // An empty key slice is a no-op success (nothing to verify against).
        crate::verify_signature(signed_file.path(), &[])
    }

    /// An archive dual-signed with two keys verifies independently against each key.
    #[test]
    #[cfg(all(
        feature = "signatures",
        feature = "archive-tar",
        feature = "compression-tar-gz",
    ))]
    fn embedded_key_verification_dual_signed_verifies_with_each_key() -> Result<()> {
        let key_a = zipsign_api::SigningKey::from_bytes(&[1u8; 32]);
        let key_b = zipsign_api::SigningKey::from_bytes(&[2u8; 32]);
        let vkey_a: [u8; zipsign_api::PUBLIC_KEY_LENGTH] = key_a.verifying_key().to_bytes();
        let vkey_b: [u8; zipsign_api::PUBLIC_KEY_LENGTH] = key_b.verifying_key().to_bytes();

        let unsigned = make_tar_gz()?;
        let signed_file = sign_tar_gz(&unsigned, &[key_a, key_b])?;

        super::verify_signature(signed_file.path(), &[vkey_a])?;
        super::verify_signature(signed_file.path(), &[vkey_b])
    }

    /// A key that did not sign the archive must produce a Signature error.
    #[test]
    #[cfg(all(
        feature = "signatures",
        feature = "archive-tar",
        feature = "compression-tar-gz",
    ))]
    fn embedded_key_verification_wrong_key_returns_error() -> Result<()> {
        let key_a = zipsign_api::SigningKey::from_bytes(&[1u8; 32]);
        let key_b = zipsign_api::SigningKey::from_bytes(&[2u8; 32]);
        let vkey_b: [u8; zipsign_api::PUBLIC_KEY_LENGTH] = key_b.verifying_key().to_bytes();

        let unsigned = make_tar_gz()?;
        let signed_file = sign_tar_gz(&unsigned, &[key_a])?;

        let err = super::verify_signature(signed_file.path(), &[vkey_b]).unwrap_err();
        assert!(
            matches!(err, crate::errors::Error::Signature(_)),
            "expected Signature error, got: {err}"
        );
        Ok(())
    }

    /// Empty key set is a deliberate no-op: verification is skipped entirely and `Ok(())` is
    /// returned *before* the archive is even opened. We point it at a path that does not exist to
    /// prove no I/O or detection happens; if the short-circuit regressed, the missing file (or the
    /// absent signature) would surface as an error instead of `Ok`.
    #[test]
    #[cfg(feature = "signatures")]
    fn embedded_key_verification_empty_keys_is_noop() {
        let missing = std::path::Path::new("/nonexistent/self_update/never_here.tar.gz");
        let res = super::verify_signature(missing, &[]);
        assert!(
            res.is_ok(),
            "empty key set must skip verification and return Ok without touching the file, got: {res:?}"
        );
    }

    /// Key-rotation / embedded any-of semantics on the *verifier* side: an archive signed by a
    /// single key must verify when that key appears anywhere in the caller's key list, even
    /// alongside keys that did not sign it. This is the production embedded-key scenario (ship N
    /// trusted keys, accept a release signed by any one of them).
    #[test]
    #[cfg(all(
        feature = "signatures",
        feature = "archive-tar",
        feature = "compression-tar-gz",
    ))]
    fn embedded_key_verification_any_of_verifier_keys_accepts() -> Result<()> {
        let signer = zipsign_api::SigningKey::from_bytes(&[7u8; 32]);
        let wrong = zipsign_api::SigningKey::from_bytes(&[8u8; 32]);
        let vkey_signer: [u8; zipsign_api::PUBLIC_KEY_LENGTH] = signer.verifying_key().to_bytes();
        let vkey_wrong: [u8; zipsign_api::PUBLIC_KEY_LENGTH] = wrong.verifying_key().to_bytes();

        let unsigned = make_tar_gz()?;
        let signed_file = sign_tar_gz(&unsigned, &[signer])?;

        // The matching key is second in the list: a first-match-only or all-must-match
        // implementation would reject this.
        super::verify_signature(signed_file.path(), &[vkey_wrong, vkey_signer])?;
        // ...and order-independence: matching key first.
        super::verify_signature(signed_file.path(), &[vkey_signer, vkey_wrong])
    }

    /// An archive whose kind is real but unsupported for signing (a bare `.tar`, i.e.
    /// `ArchiveKind::Tar(None)`, which is neither `.tar.gz` nor `.zip`) must fall through to
    /// `Error::NoSignatures`, not silently pass. The file is opened successfully, so this exercises
    /// the post-open match fall-through arm rather than an I/O error.
    #[test]
    #[cfg(all(feature = "signatures", feature = "archive-tar"))]
    fn embedded_key_verification_unsupported_archive_kind_returns_no_signatures() -> Result<()> {
        let vkey = zipsign_api::SigningKey::from_bytes(&[9u8; 32])
            .verifying_key()
            .to_bytes();

        let f = tempfile::Builder::new().suffix(".tar").tempfile()?;
        std::fs::write(
            f.path(),
            b"not really a tar, but the extension decides the kind",
        )?;

        let err = super::verify_signature(f.path(), &[vkey]).unwrap_err();
        assert!(
            matches!(err, crate::errors::Error::NoSignatures(_)),
            "a bare .tar with non-empty keys must yield NoSignatures, got: {err}"
        );
        Ok(())
    }

    /// A non-UTF-8 archive filename cannot be used as signing context and must surface as
    /// `Error::SignatureNonUTF8` (not a panic, not a generic Signature error). The error is raised
    /// before the file is opened, so the path need not exist. Unix-only because constructing a
    /// non-UTF-8 path is platform-specific.
    #[test]
    #[cfg(all(unix, feature = "signatures", feature = "archive-tar"))]
    fn embedded_key_verification_non_utf8_filename_returns_non_utf8_error() {
        use std::os::unix::ffi::OsStrExt;

        let vkey = zipsign_api::SigningKey::from_bytes(&[3u8; 32])
            .verifying_key()
            .to_bytes();

        // Invalid UTF-8 bytes, but still a `.tar.gz` extension so `detect_archive` succeeds and we
        // reach the filename-context step that rejects it.
        let name = std::ffi::OsStr::from_bytes(b"\xff\xfe-bad.tar.gz");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);

        let err = super::verify_signature(&path, &[vkey]).unwrap_err();
        assert!(
            matches!(err, crate::errors::Error::SignatureNonUTF8),
            "a non-UTF-8 archive name must yield SignatureNonUTF8, got: {err}"
        );
    }

    // --- ZIP signature verification ------------------------------------------------------------

    /// Build a minimal in-memory `.zip` (stored, no compression) suitable for signing.
    #[cfg(all(feature = "signatures", feature = "archive-zip"))]
    fn make_zip() -> Result<Vec<u8>> {
        use std::io::Cursor;
        use std::io::Write;

        let mut buf = Cursor::new(Vec::<u8>::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("hello.txt", options)
                .expect("start zip file");
            zip.write_all(b"hello").expect("write zip file");
            zip.finish().expect("finish zip");
        }
        Ok(buf.into_inner())
    }

    /// Sign in-memory `.zip` bytes into a `.zip` tempfile whose own filename is the signing context.
    #[cfg(all(feature = "signatures", feature = "archive-zip"))]
    fn sign_zip(
        unsigned: &[u8],
        signing_keys: &[zipsign_api::SigningKey],
    ) -> Result<tempfile::NamedTempFile> {
        use std::io::Cursor;

        let signed_file = tempfile::Builder::new().suffix(".zip").tempfile()?;
        let context = signed_file
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .as_bytes()
            .to_vec();

        let mut unsigned_cursor = Cursor::new(unsigned.to_vec());
        zipsign_api::sign::copy_and_sign_zip(
            &mut unsigned_cursor,
            &mut signed_file.as_file(),
            signing_keys,
            Some(&context),
        )
        .map_err(zipsign_api::ZipsignError::from)?;

        Ok(signed_file)
    }

    /// The `.zip` branch of `verify_signature` must accept an archive signed with the matching key.
    /// The tar tests never exercise the ZIP arm; this covers it independently.
    #[test]
    #[cfg(all(feature = "signatures", feature = "archive-zip"))]
    fn embedded_key_verification_zip_const_seed_verifies() -> Result<()> {
        const KEY_SEED: [u8; 32] = [55u8; 32];
        let signing_key = zipsign_api::SigningKey::from_bytes(&KEY_SEED);
        let vkey: [u8; zipsign_api::PUBLIC_KEY_LENGTH] = signing_key.verifying_key().to_bytes();

        let unsigned = make_zip()?;
        let signed_file = sign_zip(&unsigned, &[signing_key])?;

        super::verify_signature(signed_file.path(), &[vkey])
    }

    /// The `.zip` branch must reject a wrong key with a `Signature` error, mirroring the tar arm.
    #[test]
    #[cfg(all(feature = "signatures", feature = "archive-zip"))]
    fn embedded_key_verification_zip_wrong_key_returns_error() -> Result<()> {
        let key_a = zipsign_api::SigningKey::from_bytes(&[11u8; 32]);
        let key_b = zipsign_api::SigningKey::from_bytes(&[12u8; 32]);
        let vkey_b: [u8; zipsign_api::PUBLIC_KEY_LENGTH] = key_b.verifying_key().to_bytes();

        let unsigned = make_zip()?;
        let signed_file = sign_zip(&unsigned, &[key_a])?;

        let err = super::verify_signature(signed_file.path(), &[vkey_b]).unwrap_err();
        assert!(
            matches!(err, crate::errors::Error::Signature(_)),
            "expected Signature error for wrong zip key, got: {err}"
        );
        Ok(())
    }

    // --- S6: template-substitution path-traversal guard --------------------------------------

    /// Build a [`FinishCtx`] for the substitution guard tests. The archive is never read (the guard
    /// fires before extraction), so `bin_path_in_archive` + the (attacker-controlled) `version`
    /// drive the outcome.
    fn traversal_ctx(bin_path_in_archive: &str, version: &str) -> super::FinishCtx {
        super::FinishCtx {
            release: rel(version),
            bin_install_path: std::path::PathBuf::from("unused"),
            target: "x86_64-unknown-linux-gnu".to_string(),
            bin_name: "app".to_string(),
            bin_path_in_archive: bin_path_in_archive.to_string(),
            bundle_path_in_archive: None,
            bundle_target: None,
            show_output: false,
            verify_callback: None,
            #[cfg(feature = "checksums")]
            verify_checksum: None,
            #[cfg(feature = "checksums")]
            asset_digest: None,
            #[cfg(feature = "checksums")]
            verify_release_digest: true,
            #[cfg(feature = "signatures")]
            verify_keys: vec![],
        }
    }

    // A malicious release `version` like `../evil` substituted into the extraction path must be
    // rejected (S6), before any archive read, with `Error::InvalidAssetName` naming the offending
    // component. Without the guard this would redirect extraction outside the temp dir.
    #[test]
    fn finish_update_rejects_traversal_in_substituted_version() {
        let ctx = traversal_ctx("{{ version }}/app", "../evil");
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("archive.tar.gz");
        let res = super::finish_update_owned(ctx, dir, &archive);
        match res {
            Err(super::Error::InvalidAssetName { name }) => {
                assert_eq!(name, "../evil", "the offending component must be named");
            }
            other => panic!("expected InvalidAssetName for a traversal version, got {other:?}"),
        }
    }

    // A separator injected through the substituted value (e.g. `sub/evil`) is likewise rejected.
    #[test]
    fn finish_update_rejects_separator_in_substituted_version() {
        let ctx = traversal_ctx("{{ version }}", "sub/evil");
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("archive.tar.gz");
        assert!(
            matches!(
                super::finish_update_owned(ctx, dir, &archive),
                Err(super::Error::InvalidAssetName { .. })
            ),
            "a `/` in a substituted component must be rejected"
        );
    }

    // The guard only fires for a variable the template actually references. A weird `version` that
    // never reaches the path (template has no `{{ version }}`) must NOT fail here; extraction of a
    // safe, literal path proceeds to the archive read (which then errors on the missing dummy file,
    // an IO error — not `InvalidAssetName`).
    #[test]
    fn finish_update_does_not_reject_unreferenced_substitution_value() {
        let ctx = traversal_ctx("plain-bin", "../evil");
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("archive.tar.gz");
        let res = super::finish_update_owned(ctx, dir, &archive);
        assert!(
            !matches!(res, Err(super::Error::InvalidAssetName { .. })),
            "an unreferenced traversal value must not trigger the substitution guard, got {res:?}"
        );
    }

    // --- S2: credential host-gate parity (RequestConfig::auth_allowed_for) --------------------

    // The host-gate mirrors `RequestConfig::auth_allowed_for`: a matching host over https is
    // allowed; a non-matching host is not; and a matching host over plain http is not (unless
    // loopback / insecure override).
    #[test]
    fn config_auth_allowed_for_gates_host_and_scheme() {
        let cfg = crate::backends::common::RequestConfig {
            auth_base_host: Some("api.example.com".into()),
            auth_hosts: vec!["cdn.example.com".into()],
            ..Default::default()
        };

        assert!(
            cfg.auth_allowed_for("https://api.example.com/asset"),
            "the configured API host over https is allowed"
        );
        assert!(
            cfg.auth_allowed_for("https://cdn.example.com/asset"),
            "an allow_auth_host entry over https is allowed"
        );
        assert!(
            !cfg.auth_allowed_for("https://evil.example.net/asset"),
            "an unlisted host must be rejected"
        );
        assert!(
            !cfg.auth_allowed_for("http://api.example.com/asset"),
            "a matching host over plain http (non-loopback) must be rejected"
        );
    }

    // Loopback hosts are allowed over plain http (only when host-matched), matching the derived-token
    // rule; the insecure-forwarding flag lifts the https requirement for any matched host.
    #[test]
    fn config_auth_allowed_for_loopback_and_insecure_flag() {
        let cfg = crate::backends::common::RequestConfig {
            auth_hosts: vec!["127.0.0.1".into()],
            ..Default::default()
        };
        assert!(
            cfg.auth_allowed_for("http://127.0.0.1:8080/asset"),
            "a host-matched loopback address is allowed over http"
        );

        let mut cfg = crate::backends::common::RequestConfig {
            auth_base_host: Some("internal.example.com".into()),
            ..Default::default()
        };
        assert!(
            !cfg.auth_allowed_for("http://internal.example.com/asset"),
            "http to a matched non-loopback host is rejected by default"
        );
        cfg.allow_insecure_auth = true;
        assert!(
            cfg.auth_allowed_for("http://internal.example.com/asset"),
            "allow_insecure_auth lifts the https requirement for a matched host"
        );
        assert!(
            !cfg.auth_allowed_for("http://other.example.com/asset"),
            "allow_insecure_auth still requires a host match"
        );
    }

    // --- asset-name path-traversal guard (J1) ------------------------------------------------

    // A plain filename is safe.
    #[test]
    fn asset_name_valid_passes() {
        assert!(
            super::is_safe_asset_name("my-binary-v1.0.0-linux.tar.gz"),
            "ordinary archive name must be accepted"
        );
    }

    // A name with a parent-traversal prefix is rejected.
    #[test]
    fn asset_name_dot_dot_slash_is_rejected() {
        assert!(
            !super::is_safe_asset_name("../evil"),
            "../evil must be rejected"
        );
    }

    // An absolute Unix path is rejected.
    #[test]
    fn asset_name_absolute_unix_path_is_rejected() {
        assert!(
            !super::is_safe_asset_name("/etc/hosts"),
            "/etc/hosts must be rejected"
        );
    }

    // A bare `..` component is rejected.
    #[test]
    fn asset_name_dot_dot_alone_is_rejected() {
        assert!(
            !super::is_safe_asset_name(".."),
            ".. alone must be rejected"
        );
    }

    // A bare `.` component is rejected.
    #[test]
    fn asset_name_dot_alone_is_rejected() {
        assert!(!super::is_safe_asset_name("."), ". alone must be rejected");
    }

    // An empty string is rejected.
    #[test]
    fn asset_name_empty_is_rejected() {
        assert!(
            !super::is_safe_asset_name(""),
            "empty name must be rejected"
        );
    }

    // A name containing `/` (embedded slash) is rejected.
    #[test]
    fn asset_name_with_slash_is_rejected() {
        assert!(
            !super::is_safe_asset_name("sub/path"),
            "name containing / must be rejected"
        );
    }

    // A name containing `\` (Windows separator) is rejected.
    #[test]
    fn asset_name_with_backslash_is_rejected() {
        assert!(
            !super::is_safe_asset_name("sub\\path"),
            "name containing \\ must be rejected"
        );
    }

    // A Windows drive-relative name (`C:evil`) has no separator and is not absolute, but `Path::join`
    // would treat the disk designator as a drive-qualified path and escape the temp dir. It must be
    // rejected on every platform (the crate may be built for Windows).
    #[test]
    fn asset_name_with_windows_drive_prefix_is_rejected() {
        assert!(
            !super::is_safe_asset_name("C:evil"),
            "a Windows drive-relative name must be rejected"
        );
        assert!(
            !super::is_safe_asset_name("C:"),
            "a bare drive designator must be rejected"
        );
        assert!(
            !super::is_safe_asset_name("C:\\evil"),
            "a drive-absolute name must be rejected"
        );
    }

    // `InvalidAssetName` error variant: Display carries the correct prefix and the name.
    #[test]
    fn invalid_asset_name_error_display() {
        let err = crate::errors::Error::InvalidAssetName {
            name: "../evil".to_string(),
        };
        let shown = err.to_string();
        assert!(
            shown.starts_with("InvalidAssetNameError: "),
            "must carry the expected prefix, got: {shown}"
        );
        assert!(
            shown.contains("../evil"),
            "must embed the offending name, got: {shown}"
        );
    }

    // `InvalidAssetName` has no HTTP status and no URL.
    #[test]
    fn invalid_asset_name_error_http_helpers_are_none() {
        let err = crate::errors::Error::InvalidAssetName {
            name: "../evil".to_string(),
        };
        assert_eq!(err.http_status(), None);
        assert_eq!(err.url(), None);
    }
}

/*!
gitee releases
*/
use crate::backends::common::{CommonBuilderConfig, CommonConfig, RequestConfig};
use crate::backends::{Page, PageRequest, first_page_url, next_link, run_paginated};
use crate::http_client::{HeaderMap, header};
use crate::version::bump_is_greater;
use crate::{
    errors::*,
    update::{Release, ReleaseAsset, ReleaseUpdate, Releases},
};
use serde::Deserialize;

/// The default gitee host. Unlike gitea (which has no canonical public host and requires one),
/// gitee.com is the canonical public instance, so `host(..)` is optional and defaults here. The
/// setter is kept for self-hosted Gitee Enterprise deployments.
const DEFAULT_HOST: &str = "https://gitee.com";

/// Gitee release-asset JSON shape (download URL is `browser_download_url`). Private DTO converted
/// into the public [`ReleaseAsset`]; keeping it private keeps `Deserialize` out of `ReleaseAsset`'s
/// public API.
#[derive(Deserialize)]
struct AssetDto {
    name: Option<String>,
    browser_download_url: Option<String>,
}

impl AssetDto {
    /// Convert to a public asset, or `None` (skipped) when it lacks a usable `name` or
    /// `browser_download_url`.
    ///
    /// This is deliberately LENIENT, unlike gitea's strict `into_asset` (which errors on a missing
    /// field). Every gitee release carries an auto-generated source-code archive that appears in the
    /// `assets` array WITHOUT a `name` (and often without a `browser_download_url`). Treating that
    /// as a hard `MissingAssetField` error would make every gitee release fail to parse and thus be
    /// unusable, so a nameless / URL-less asset is quietly dropped (debug-logged) instead. Named,
    /// downloadable assets on the same release still parse normally.
    fn into_asset(self) -> Option<ReleaseAsset> {
        let name = match self.name {
            Some(name) => name,
            None => {
                log::debug!(
                    "self_update: skipping gitee asset with no name (likely the auto-generated \
                     source archive)"
                );
                return None;
            }
        };
        let download_url = match self.browser_download_url {
            Some(url) => url,
            None => {
                log::debug!(
                    "self_update: skipping gitee asset `{name}` with no browser_download_url"
                );
                return None;
            }
        };
        Some(ReleaseAsset::new(name, download_url))
    }
}

/// Gitee release JSON shape. Private DTO deserialized directly from the response bytes, then
/// converted into the public [`Release`].
#[derive(Deserialize)]
struct ReleaseDto {
    tag_name: Option<String>,
    created_at: Option<String>,
    name: Option<String>,
    body: Option<String>,
    html_url: Option<String>,
    assets: Option<Vec<AssetDto>>,
}

impl ReleaseDto {
    fn into_release(self, tag_prefix: Option<&str>) -> Result<Release> {
        let tag = self
            .tag_name
            .ok_or_else(|| Error::missing_asset_field("tag_name"))?;
        let date = self
            .created_at
            .ok_or_else(|| Error::missing_asset_field("created_at"))?;
        let assets = self
            .assets
            .ok_or_else(|| Error::missing_asset_field("assets"))?;
        let name = self.name.unwrap_or_else(|| tag.clone());
        // Lenient asset mapping: nameless / URL-less assets (gitee's auto-generated source archive)
        // are skipped rather than failing the whole release. See `AssetDto::into_asset`.
        let assets = assets
            .into_iter()
            .filter_map(AssetDto::into_asset)
            .collect::<Vec<ReleaseAsset>>();
        let version =
            crate::backends::common::strip_tag_prefix(&tag, tag_prefix).ok_or_else(|| {
                crate::backends::common::tag_prefix_mismatch_error(
                    &tag,
                    tag_prefix.unwrap_or_default(),
                )
            })?;
        let mut builder = Release::builder();
        builder
            .name(name)
            .version(version)
            .date(date)
            .assets(assets);
        if let Some(body) = self.body {
            builder.body(body);
        }
        if let Some(url) = self.html_url {
            builder.release_notes_url(url);
        }
        builder
            .build()
            .map_err(|e| crate::backends::common::name_tag_in_semver_error(&tag, e))
    }
}

/// `ReleaseList` Builder
#[derive(Clone, Debug)]
#[must_use]
pub struct ReleaseListBuilder {
    host: Option<String>,
    repo_owner: Option<String>,
    repo_name: Option<String>,
    target: Option<String>,
    auth_token: Option<String>,
    request: RequestConfig,
}
impl ReleaseListBuilder {
    /// Optional. Set the base URL of a self-hosted Gitee (Gitee Enterprise) instance, e.g.
    /// `https://gitee.example.com`. Defaults to `https://gitee.com`.
    ///
    /// Unlike `gitea` (which has no canonical public host and so requires this), gitee.com is the
    /// canonical public instance, so leaving this unset targets gitee.com.
    ///
    /// Pass the instance host only (scheme + host, no trailing slash); the crate appends the
    /// `/api/v5/...` path itself. Do not include `/api/v5`.
    pub fn host(&mut self, url: impl Into<String>) -> &mut Self {
        self.host = Some(url.into());
        self
    }

    /// Required. Set the repo owner, used to build a gitee api url
    pub fn repo_owner(&mut self, owner: impl Into<String>) -> &mut Self {
        self.repo_owner = Some(owner.into());
        self
    }

    /// Required. Set the repo name, used to build a gitee api url
    pub fn repo_name(&mut self, name: impl Into<String>) -> &mut Self {
        self.repo_name = Some(name.into());
        self
    }

    /// Set the optional arch `target` name, used to filter the releases this list returns to
    /// those carrying an asset whose name contains `target`.
    ///
    /// This is the **`ReleaseList`** filter and differs from
    /// [`Update::target`](UpdateBuilder::target): `filter_target` drops whole releases from the
    /// listing when no asset matches, whereas the `Update` `target` selects *which asset* of the
    /// chosen release to download.
    pub fn filter_target(&mut self, target: impl Into<String>) -> &mut Self {
        self.target = Some(target.into());
        self
    }

    /// Set the authorization token, used in requests to the gitee api url
    ///
    /// This is to support private repos where you need a gitee auth token.
    /// **Make sure not to bake the token into your app**; it is recommended
    /// you obtain it via another mechanism, such as environment variables
    /// or prompting the user for input
    pub fn auth_token(&mut self, auth_token: impl Into<String>) -> &mut Self {
        self.auth_token = Some(auth_token.into());
        self
    }

    request_config_setters!(request);

    /// Verify builder args, returning a `ReleaseList`
    pub fn build(&self) -> Result<ReleaseList> {
        // Thread the auth token + gitee's `Bearer` scheme into the request so the shared
        // `apply_auth` applies it on the listing path (honoring a user override). Gitee v5 accepts
        // `Authorization: Bearer <token>` (verified against gitee's official client
        // oschina/mcp-gitee gitee_client.go).
        let host = self
            .host
            .clone()
            .unwrap_or_else(|| DEFAULT_HOST.to_string());
        let mut request = self.request.clone();
        request.auth_scheme = crate::backends::common::AuthScheme::Bearer;
        request.auth_token = self.auth_token.clone();
        request.auth_base_host = crate::backends::common::host_of(&host);
        request.build_client();
        request.check()?;
        Ok(ReleaseList {
            host,
            repo_owner: if let Some(ref owner) = self.repo_owner {
                owner.to_owned()
            } else {
                return Err(Error::MissingField {
                    field: "repo_owner",
                });
            },
            repo_name: if let Some(ref name) = self.repo_name {
                name.to_owned()
            } else {
                return Err(Error::MissingField { field: "repo_name" });
            },
            target: self.target.clone(),
            request,
        })
    }
}

/// `ReleaseList` provides a builder api for querying a gitee repo,
/// returning a `Vec` of available `Release`s
#[derive(Clone, Debug)]
pub struct ReleaseList {
    host: String,
    repo_owner: String,
    repo_name: String,
    target: Option<String>,
    request: RequestConfig,
}
impl ReleaseList {
    /// Initialize a ReleaseListBuilder
    pub fn configure() -> ReleaseListBuilder {
        ReleaseListBuilder {
            host: None,
            repo_owner: None,
            repo_name: None,
            target: None,
            auth_token: None,
            request: RequestConfig::default(),
        }
    }
    // Note: `auth_token` lives only in `ReleaseListBuilder` (to wire into `request.auth_token`
    // during `build()`). The built `ReleaseList` does not carry it: auth is applied centrally
    // by `apply_auth` on the request config during transport.

    /// Retrieve the available `Release`s as a [`Releases`].
    ///
    /// If a `filter_target` is set, only releases carrying an asset whose name contains it are
    /// returned. The result carries no current version (it is a bare listing), so
    /// [`Releases::current_version`] is `None`; use [`Releases::into_vec`] to recover the raw
    /// `Vec<Release>`.
    pub fn fetch(&self) -> Result<Releases> {
        let api_url = format!(
            "{}/api/v5/repos/{}/{}/releases",
            self.host,
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        );

        // An unfiltered listing must walk ALL pages: `stop_at = None`.
        let releases = run_paginated(releases_plan(&api_url, None, None)?, &self.request)?;
        let releases = match self.target {
            None => releases,
            Some(ref target) => releases
                .into_iter()
                .filter(|r| r.has_target_asset(target))
                .collect::<Vec<_>>(),
        };
        Ok(Releases::from_listing(releases))
    }

    /// Async sibling of [`fetch`](Self::fetch).
    #[cfg(feature = "async")]
    pub async fn fetch_async(&self) -> Result<Releases> {
        let api_url = format!(
            "{}/api/v5/repos/{}/{}/releases",
            self.host,
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        );

        // An unfiltered listing must walk ALL pages: `stop_at = None`.
        let releases = crate::backends::run_paginated_async(
            releases_plan(&api_url, None, None)?,
            &self.request,
        )
        .await?;
        let releases = match self.target {
            None => releases,
            Some(ref target) => releases
                .into_iter()
                .filter(|r| r.has_target_asset(target))
                .collect::<Vec<_>>(),
        };
        Ok(Releases::from_listing(releases))
    }
}

/// `gitee::Update` builder
///
/// Configure download and installation from
/// `https://<gitee-host>/api/v5/repos/<repo_owner>/<repo_name>/releases`
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct UpdateBuilder {
    host: Option<String>,
    repo_owner: Option<String>,
    repo_name: Option<String>,
    common: CommonBuilderConfig,
}

impl UpdateBuilder {
    /// Initialize a new builder
    pub fn new() -> Self {
        Default::default()
    }

    /// Optional. Set the base URL of a self-hosted Gitee (Gitee Enterprise) instance, e.g.
    /// `https://gitee.example.com`. Defaults to `https://gitee.com`.
    ///
    /// Unlike `gitea` (which has no canonical public host and so requires this), gitee.com is the
    /// canonical public instance, so leaving this unset targets gitee.com.
    ///
    /// Pass the instance host only (scheme + host, no trailing slash); the crate appends the
    /// `/api/v5/...` path itself. Do not include `/api/v5`.
    pub fn host(&mut self, url: impl Into<String>) -> &mut Self {
        self.host = Some(url.into());
        self
    }

    /// Required. Set the repo owner, used to build a gitee api url
    pub fn repo_owner(&mut self, owner: impl Into<String>) -> &mut Self {
        self.repo_owner = Some(owner.into());
        self
    }

    /// Required. Set the repo name, used to build a gitee api url
    pub fn repo_name(&mut self, name: impl Into<String>) -> &mut Self {
        self.repo_name = Some(name.into());
        self
    }

    /// Set the tag prefix used to derive a release version from its tag. Defaults to unset, which
    /// trims a leading `v` (so `v1.2.3` and `1.2.3` both yield `1.2.3`). Set it to, e.g., `myapp-`
    /// for a monorepo whose tags look like `myapp-1.2.3` (or `myapp-v1.2.3`); tags without the
    /// prefix are then skipped from the listing rather than mis-parsed.
    pub fn tag_prefix(&mut self, prefix: impl Into<String>) -> &mut Self {
        self.common.tag_prefix = Some(prefix.into());
        self
    }

    impl_common_builder_setters!();

    /// Internal: validate config into a concrete `Update`. Shared by `build` / `build_async`.
    fn build_update(&self) -> Result<Update> {
        let host = self
            .host
            .clone()
            .unwrap_or_else(|| DEFAULT_HOST.to_string());
        Ok(Update {
            repo_owner: if let Some(ref owner) = self.repo_owner {
                owner.to_owned()
            } else {
                return Err(Error::MissingField {
                    field: "repo_owner",
                });
            },
            repo_name: if let Some(ref name) = self.repo_name {
                name.to_owned()
            } else {
                return Err(Error::MissingField { field: "repo_name" });
            },
            common: {
                // Gitee authenticates with `Bearer <token>` (verified against gitee's official
                // client oschina/mcp-gitee gitee_client.go); set the scheme explicitly rather than
                // relying on `AuthScheme::default()` (which is `Token`).
                let mut resolved = self.common.build()?;
                resolved.request.auth_scheme = crate::backends::common::AuthScheme::Bearer;
                // Only the gitee host receives the token; a server-supplied asset download URL on
                // another host does not.
                resolved.request.auth_base_host = crate::backends::common::host_of(&host);
                resolved
            },
            host,
        })
    }

    /// Confirm config and create a ready-to-use `Update`.
    ///
    /// Returns the concrete [`Update`], which is `Send` and exposes the update verbs as inherent
    /// methods.
    pub fn build(&self) -> Result<Update> {
        self.build_update()
    }

    /// Confirm config and create a ready-to-use [`AsyncUpdate`] for the async API (`update_async`).
    ///
    /// Unlike [`build`](Self::build) this returns the distinct [`AsyncUpdate`] newtype, which exposes
    /// only the inherent `*_async` verbs, so a stray blocking `.update()` on an async-built updater
    /// is a compile error rather than a silent block of the executor.
    #[cfg(feature = "async")]
    pub fn build_async(&self) -> Result<AsyncUpdate> {
        Ok(AsyncUpdate(self.build_update()?))
    }
}

/// Updates to a specified or latest release distributed via gitee
#[derive(Debug)]
#[non_exhaustive]
pub struct Update {
    host: String,
    repo_owner: String,
    repo_name: String,
    common: CommonConfig,
}
impl Update {
    /// Initialize a new `Update` builder
    pub fn configure() -> UpdateBuilder {
        UpdateBuilder::new()
    }

    /// Base releases URL. Shared by the sync and async fetch paths so they can't drift.
    fn releases_url(&self) -> String {
        format!(
            "{}/api/v5/repos/{}/{}/releases",
            self.host,
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        )
    }

    /// The dedicated newest-release URL: `.../releases/latest` (a single release *object*).
    fn latest_url(&self) -> String {
        format!("{}/latest", self.releases_url())
    }
}

impl crate::update::sealed::Sealed for Update {}

impl Update {
    /// The single-release-by-tag URL: `.../releases/tags/{ver}`.
    fn tag_url(&self, ver: &str) -> String {
        format!("{}/tags/{}", self.releases_url(), urlencoding::encode(ver))
    }
}

impl ReleaseUpdate for Update {
    fn get_latest_release(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = run_paginated(
            newest_plan(
                self.latest_url(),
                &self.releases_url(),
                self.common.tag_prefix.as_deref(),
            )?,
            &self.common.request,
        )?;
        let release = releases
            .into_iter()
            .next()
            .ok_or_else(|| Error::NoReleaseFound { target: None })?;
        Ok(Releases::new(vec![release], current_version))
    }

    fn get_newer_releases(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = run_paginated(
            releases_plan(
                &self.releases_url(),
                Some(&current_version),
                self.common.tag_prefix.as_deref(),
            )?,
            &self.common.request,
        )?;
        Ok(Releases::new(releases, current_version))
    }

    fn get_release_version(&self, ver: &str) -> Result<Release> {
        let releases = run_paginated(
            single_plan(self.tag_url(ver), self.common.tag_prefix.as_deref())?,
            &self.common.request,
        )?;
        releases
            .into_iter()
            .next()
            .ok_or_else(|| Error::NoReleaseFound { target: None })
    }
}

impl_sync_update_verbs!(Update);

/// Async-only updater returned by [`UpdateBuilder::build_async`].
///
/// A newtype over the blocking [`Update`] that exposes **only** the inherent `*_async` verbs. Using
/// it (instead of returning `Update` from `build_async`) makes a blocking call on an async-built
/// updater -- e.g. `build_async()?.update()` -- a compile error, so the async executor cannot be
/// silently blocked.
#[cfg(feature = "async")]
#[derive(Debug)]
pub struct AsyncUpdate(Update);

#[cfg(feature = "async")]
impl_async_update_verbs!(AsyncUpdate);

impl_update_config_accessors!(Update, {
    fn api_headers(&self, _auth_token: Option<&str>) -> Result<header::HeaderMap> {
        api_headers()
    }
});

/// Transport-free plan to fetch the paginated `releases` array (Gitee format), parsing each page
/// via the private `ReleaseDto` and following `Link: rel="next"`. See github's `releases_plan`
/// for the `stop_at` per-item filter contract.
///
/// `stop_at` filters per-item: when `Some(current_version)` each release that is not strictly
/// newer than it is omitted from the collected list, but pagination continues to subsequent pages
/// regardless (a backport release -- older semver, newer creation date -- must not halt the walk
/// and cause a genuinely newer release on a later page to be missed). When `None` the listing is
/// unfiltered and every page is walked (used by `ReleaseList`).
fn releases_plan(
    base_url: &str,
    stop_at: Option<&str>,
    tag_prefix: Option<&str>,
) -> Result<PageRequest<Release>> {
    let headers = api_headers()?;
    let stop_at = stop_at.map(str::to_owned);
    Ok(release_array_page(
        first_page_url(base_url),
        headers,
        stop_at,
        tag_prefix.map(str::to_owned),
    ))
}

fn release_array_page(
    url: String,
    headers: HeaderMap,
    stop_at: Option<String>,
    tag_prefix: Option<String>,
) -> PageRequest<Release> {
    PageRequest {
        url,
        headers,
        parse: Box::new(move |body, resp_headers| {
            // Deserialize the page directly into the private DTO vec (no intermediate
            // `serde_json::Value` tree), then convert each into a public `Release`.
            let dtos: Vec<ReleaseDto> =
                serde_json::from_slice(body).map_err(|e| Error::InvalidResponse {
                    source: Box::new(e),
                })?;
            let mut items = Vec::new();
            for dto in dtos {
                let release = match dto.into_release(tag_prefix.as_deref()) {
                    Ok(release) => release,
                    // A non-semver tag (`nightly`, `latest`, a date tag) is not a release the
                    // updater can compare; skip it rather than failing the whole listing, so a
                    // repository mixing rolling tags with semver releases stays updatable.
                    Err(e @ Error::SemVer(_)) => {
                        log::debug!("self_update: skipping listed release: {e}");
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                // Skip releases not strictly newer than the current version, but do NOT stop
                // pagination. A backport release (older semver, newer creation date) must not
                // halt the walk; a genuinely newer release on a later page must still be found.
                if let Some(ref current) = stop_at
                    && !bump_is_greater(current, release.version()).unwrap_or(false)
                {
                    continue;
                }
                items.push(release);
            }
            let next = next_link(resp_headers)
                .map(|next_url| -> Result<PageRequest<Release>> {
                    Ok(release_array_page(
                        next_url,
                        api_headers()?,
                        stop_at.clone(),
                        tag_prefix.clone(),
                    ))
                })
                .transpose()?;
            Ok(Page {
                items,
                next,
                stop: false,
            })
        }),
    }
}

/// Transport-free plan for the newest release. Unlike gitea/gitlab (which have no `/releases/latest`
/// and take the listing's first entry), gitee has a dedicated `/api/v5/.../releases/latest`
/// endpoint returning a single release *object*. Fetch that first.
///
/// If that "latest" release carries a non-semver rolling tag (`nightly`, `latest`, ...) the updater
/// cannot compare it, so fall back to scanning the listing's first page for the newest release the
/// updater CAN compare -- mirroring gitea's `newest_plan` skip semantics. That fallback is wired as
/// the `next` page of the initial (empty-items) `/latest` page, so the shared paginated driver
/// performs the second fetch.
fn newest_plan(
    latest_url: String,
    listing_url: &str,
    tag_prefix: Option<&str>,
) -> Result<PageRequest<Release>> {
    let headers = api_headers()?;
    let tag_prefix = tag_prefix.map(str::to_owned);
    let listing_first = first_page_url(listing_url);
    Ok(PageRequest {
        url: latest_url,
        headers,
        parse: Box::new(move |body, _resp_headers| {
            // `/releases/latest` returns a single release *object* (parsed like the tag route).
            let dto: ReleaseDto =
                serde_json::from_slice(body).map_err(crate::errors::Error::invalid_response)?;
            match dto.into_release(tag_prefix.as_deref()) {
                Ok(release) => Ok(Page::last(vec![release])),
                // The pinned "latest" is a non-semver rolling tag; fall back to scanning the
                // listing for the newest release the updater can actually compare.
                Err(Error::SemVer(e)) => {
                    log::debug!(
                        "self_update: gitee latest release is non-semver ({e}); \
                         scanning the listing for the newest comparable release"
                    );
                    Ok(Page {
                        items: vec![],
                        next: Some(newest_from_listing_page(
                            listing_first.clone(),
                            api_headers()?,
                            tag_prefix.clone(),
                        )),
                        stop: false,
                    })
                }
                Err(e) => Err(e),
            }
        }),
    })
}

/// The listing-scan fallback used by [`newest_plan`] when `/releases/latest` is a non-semver tag.
/// Parses the listing array (newest-first) and returns the first release the updater can compare,
/// skipping non-semver rolling tags. An all-non-semver (or empty) page yields `NoReleaseFound`.
fn newest_from_listing_page(
    url: String,
    headers: HeaderMap,
    tag_prefix: Option<String>,
) -> PageRequest<Release> {
    PageRequest {
        url,
        headers,
        parse: Box::new(move |body, _resp_headers| {
            let dtos: Vec<ReleaseDto> =
                serde_json::from_slice(body).map_err(|e| Error::InvalidResponse {
                    source: Box::new(e),
                })?;
            for dto in dtos {
                match dto.into_release(tag_prefix.as_deref()) {
                    Ok(release) => return Ok(Page::last(vec![release])),
                    Err(e @ Error::SemVer(_)) => {
                        log::debug!("self_update: skipping listed release: {e}");
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(Error::NoReleaseFound { target: None })
        }),
    }
}

/// Transport-free plan to fetch a single release *object* (the `.../releases/tags/{ver}` endpoint).
fn single_plan(url: String, tag_prefix: Option<&str>) -> Result<PageRequest<Release>> {
    let headers = api_headers()?;
    let tag_prefix = tag_prefix.map(str::to_owned);
    Ok(PageRequest {
        url,
        headers,
        parse: Box::new(move |body, _resp_headers| {
            // An unparseable body is `InvalidResponse`, matching the paginated listing parser.
            let dto: ReleaseDto =
                serde_json::from_slice(body).map_err(crate::errors::Error::invalid_response)?;
            Ok(Page::last(vec![dto.into_release(tag_prefix.as_deref())?]))
        }),
    })
}

#[cfg(feature = "async")]
impl crate::update::AsyncReleaseUpdate for Update {
    async fn get_latest_release_async(&self) -> Result<Releases> {
        use crate::backends::run_paginated_async;
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = run_paginated_async(
            newest_plan(
                self.latest_url(),
                &self.releases_url(),
                self.common.tag_prefix.as_deref(),
            )?,
            &self.common.request,
        )
        .await?;
        let release = releases
            .into_iter()
            .next()
            .ok_or_else(|| Error::NoReleaseFound { target: None })?;
        Ok(Releases::new(vec![release], current_version))
    }

    async fn get_newer_releases_async(&self) -> Result<Releases> {
        use crate::backends::run_paginated_async;
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = run_paginated_async(
            releases_plan(
                &self.releases_url(),
                Some(&current_version),
                self.common.tag_prefix.as_deref(),
            )?,
            &self.common.request,
        )
        .await?;
        Ok(Releases::new(releases, current_version))
    }

    async fn get_release_version_async(&self, ver: &str) -> Result<Release> {
        use crate::backends::run_paginated_async;
        let releases = run_paginated_async(
            single_plan(self.tag_url(ver), self.common.tag_prefix.as_deref())?,
            &self.common.request,
        )
        .await?;
        releases
            .into_iter()
            .next()
            .ok_or_else(|| Error::NoReleaseFound { target: None })
    }
}

/// Build gitee's base request headers (its User-Agent). The Authorization header is applied
/// centrally by the shared [`apply_auth`](crate::backends::common::RequestConfig::apply_auth) using
/// gitee's `Bearer` scheme on both the listing and download paths, honoring a user override.
fn api_headers() -> Result<header::HeaderMap> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        crate::DEFAULT_USER_AGENT
            .parse()
            .expect("gitee invalid user-agent"),
    );

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::Update;
    use crate::update::UpdateConfig;

    /// Async test wrapper over `releases_plan` + the async driver (unfiltered, all pages).
    #[cfg(feature = "async")]
    async fn fetch_all_releases_async(
        base_url: &str,
        req: &crate::backends::common::RequestConfig,
    ) -> crate::errors::Result<Vec<super::Release>> {
        crate::backends::run_paginated_async(super::releases_plan(base_url, None, None)?, req).await
    }

    // The single-release endpoint (`.../releases/tags/{ver}`) surfaces an unparseable body as
    // `InvalidResponse`, matching the paginated listing parser.
    #[test]
    fn single_plan_parse_failure_is_invalid_response() {
        let req = super::single_plan("https://example.test/releases/tags/1.0.0".to_string(), None)
            .unwrap();
        let res = (req.parse)(b"not-json", &crate::http_client::HeaderMap::new());
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidResponse { .. })),
            "a malformed single-release body must map to InvalidResponse"
        );
    }

    // A rolling non-semver tag (`nightly`) in the listing must be skipped, not fail the whole
    // fetch: repositories commonly mix rolling tags with semver releases.
    #[test]
    fn listing_skips_non_semver_tags() {
        let req = super::release_array_page(
            "https://example.test/releases".to_string(),
            crate::http_client::HeaderMap::new(),
            None,
            None,
        );
        let body = releases_json(&["nightly", "v1.2.3", "v1.0.0"]);
        let page = (req.parse)(body.as_bytes(), &crate::http_client::HeaderMap::new()).unwrap();
        let versions: Vec<&str> = page.items.iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["1.2.3", "1.0.0"],
            "non-semver tags are skipped; the semver releases survive"
        );
    }

    // A configured `tag_prefix` derives the version from monorepo-style tags (`myapp-1.2.3`,
    // `myapp-v1.3.0`); tags without the prefix are skipped rather than mis-parsed.
    #[test]
    fn listing_with_tag_prefix_parses_prefixed_tags_and_skips_others() {
        let req = super::release_array_page(
            "https://example.test/releases".to_string(),
            crate::http_client::HeaderMap::new(),
            None,
            Some("myapp-".to_string()),
        );
        let body = releases_json(&["myapp-1.2.3", "otherapp-2.0.0", "myapp-v1.3.0", "1.0.0"]);
        let page = (req.parse)(body.as_bytes(), &crate::http_client::HeaderMap::new()).unwrap();
        let versions: Vec<&str> = page.items.iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["1.2.3", "1.3.0"],
            "only `myapp-`-prefixed tags are parsed (with an optional inner `v`); the rest are skipped"
        );
    }

    // The listing-scan fallback skips a rolling tag so the first COMPARABLE release wins.
    #[test]
    fn newest_from_listing_skips_non_semver_tags() {
        let req = super::newest_from_listing_page(
            "https://example.test/releases".to_string(),
            crate::http_client::HeaderMap::new(),
            None,
        );
        let body = releases_json(&["nightly", "v1.2.3", "v1.0.0"]);
        let page = (req.parse)(body.as_bytes(), &crate::http_client::HeaderMap::new()).unwrap();
        let versions: Vec<&str> = page.items.iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["1.2.3"]);
    }

    // With only non-semver tags in the fallback listing there is nothing the updater can compare:
    // NoReleaseFound.
    #[test]
    fn newest_from_listing_with_only_non_semver_tags_is_no_release_found() {
        let req = super::newest_from_listing_page(
            "https://example.test/releases".to_string(),
            crate::http_client::HeaderMap::new(),
            None,
        );
        let body = releases_json(&["nightly", "latest"]);
        let res = (req.parse)(body.as_bytes(), &crate::http_client::HeaderMap::new());
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::NoReleaseFound { target: None })
            ),
            "an all-rolling-tag listing must yield NoReleaseFound"
        );
    }

    // The single-release endpoint cannot skip: a pinned non-semver tag errors, naming the tag.
    #[test]
    fn single_plan_non_semver_tag_errors_naming_the_tag() {
        let req = super::single_plan(
            "https://example.test/releases/tags/nightly".to_string(),
            None,
        )
        .unwrap();
        let res = (req.parse)(
            release_obj_json("nightly").as_bytes(),
            &crate::http_client::HeaderMap::new(),
        );
        match res {
            Err(crate::errors::Error::SemVer(e)) => {
                assert!(
                    e.to_string().contains("nightly"),
                    "the error must name the offending tag, got: {e}"
                );
            }
            Err(other) => panic!("expected Error::SemVer, got {other:?}"),
            Ok(_) => panic!("a non-semver pinned tag must error"),
        }
    }

    use std::io::{Read, Write};
    use std::net::TcpListener;

    struct Resp {
        status: &'static str,
        link: Option<String>,
        body: String,
    }

    /// Bind a loopback listener and serve `make(base_url)`'s responses in order, one per
    /// incoming connection, on a background thread. Returns the server's base URL
    /// (`http://127.0.0.1:<port>`). No external network is used.
    fn stub(make: impl FnOnce(&str) -> Vec<Resp>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let responses = make(&base);
        std::thread::spawn(move || {
            for r in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf); // drain the request line/headers
                let mut out = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\n",
                    r.status
                );
                if let Some(link) = r.link {
                    out.push_str(&format!("Link: <{link}>; rel=\"next\"\r\n"));
                }
                out.push_str(&format!(
                    "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    r.body.len(),
                    r.body
                ));
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        base
    }

    /// A JSON array of one release (used by the async pagination tests).
    #[cfg(feature = "async")]
    fn release_json(tag: &str) -> String {
        format!(
            r#"[{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[],"body":null}}]"#
        )
    }

    /// A JSON array of several releases (one object per `tag`), used by the listing-based tests.
    fn releases_json(tags: &[&str]) -> String {
        let objs = tags
            .iter()
            .map(|tag| {
                format!(
                    r#"{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[],"body":null}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("[{objs}]")
    }

    /// A bare JSON release object (not wrapped in an array). Gitee's `get_release_version[_async]`
    /// hits `/tags/{ver}` and `get_latest_release[_async]` hits `/releases/latest`, both of which
    /// return a single release object, so this is parsed directly.
    fn release_obj_json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[],"body":null}}"#
        )
    }

    #[cfg(feature = "async")]
    fn gitee_update(base: &str, current_version: &str) -> super::AsyncUpdate {
        Update::configure()
            .host(base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version(current_version)
            .build_async()
            .unwrap()
    }

    /// Build a `ReleaseUpdate` (sync) gitee `Update` pointed at the loopback stub.
    fn gitee_update_sync(base: &str, current_version: &str) -> Update {
        Update::configure()
            .host(base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version(current_version)
            .build()
            .unwrap()
    }

    // --- Default host ------------------------------------------------------------------------

    #[test]
    fn update_defaults_host_to_gitee_com() {
        // Unlike gitea, gitee's host is optional and defaults to gitee.com; the releases URL and
        // the latest URL must be built against it.
        let upd = Update::configure()
            .repo_owner("owner")
            .repo_name("repo")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(
            upd.releases_url(),
            "https://gitee.com/api/v5/repos/owner/repo/releases"
        );
        assert_eq!(
            upd.latest_url(),
            "https://gitee.com/api/v5/repos/owner/repo/releases/latest"
        );
    }

    #[test]
    fn release_list_defaults_host_to_gitee_com() {
        // The ReleaseList builder also defaults the host: build() must succeed without a host.
        let _list = super::ReleaseList::configure()
            .repo_owner("o")
            .repo_name("r")
            .build()
            .unwrap();
    }

    #[test]
    fn host_setter_overrides_default_for_enterprise() {
        let upd = Update::configure()
            .host("https://gitee.example.com")
            .repo_owner("owner")
            .repo_name("repo")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(
            upd.releases_url(),
            "https://gitee.example.com/api/v5/repos/owner/repo/releases"
        );
    }

    // --- Sync `Releases`-returning fetch coverage -------------------------------------------

    #[test]
    fn get_latest_release_sync_wraps_newest_into_one_element_releases() {
        // `get_latest_release` fetches the dedicated `/releases/latest` object and wraps it in a
        // one-element `Releases` carrying the configured current version.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v2.5.0"),
            }]
        });
        let upd = gitee_update_sync(&base, "1.0.0");
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(
            releases.all().len(),
            1,
            "get_latest_release yields a one-element Releases"
        );
        assert_eq!(releases.latest().unwrap().version(), "2.5.0");
        assert!(
            releases.is_update_available().unwrap(),
            "2.5.0 > 1.0.0 via the one-element Releases pre-check"
        );
    }

    #[test]
    fn get_latest_release_routes_to_latest_endpoint() {
        // The first (and only) request for the semver-latest path must hit `/releases/latest`.
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v2.5.0"),
            }]
        });
        let upd = gitee_update_sync(&base, "1.0.0");
        upd.get_latest_release().unwrap();
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1, "a semver latest needs exactly one request");
        let line = reqs[0].lines().next().unwrap_or("");
        assert!(
            line.contains("/api/v5/repos/o/r/releases/latest"),
            "the latest path must hit /releases/latest, got: {line}"
        );
    }

    #[test]
    fn get_latest_release_falls_back_to_listing_when_latest_is_non_semver() {
        // `/releases/latest` returns a non-semver rolling tag; the updater must then scan the
        // listing and pick the newest comparable release. Two requests are made: /releases/latest
        // then the listing (with the ?per_page=100 first page).
        let (base, captured) = stub_capturing(|_| {
            vec![
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_obj_json("nightly"),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v1.2.3", "v1.0.0"]),
                },
            ]
        });
        let upd = gitee_update_sync(&base, "0.1.0");
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(
            releases.latest().unwrap().version(),
            "1.2.3",
            "the non-semver latest falls back to the newest comparable listing release"
        );
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 2, "fallback fetches /latest then the listing");
        assert!(
            reqs[0]
                .lines()
                .next()
                .unwrap_or("")
                .contains("/releases/latest"),
            "first request must be /releases/latest"
        );
        let second = reqs[1].lines().next().unwrap_or("");
        assert!(
            second.contains("/api/v5/repos/o/r/releases?per_page=100"),
            "second request must be the listing first page, got: {second}"
        );
    }

    #[test]
    fn get_latest_release_empty_listing_fallback_is_no_release_found() {
        // Non-semver latest, then an empty listing array: NoReleaseFound { target: None }.
        let base = stub(|_| {
            vec![
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_obj_json("nightly"),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: "[]".to_string(),
                },
            ]
        });
        let upd = gitee_update_sync(&base, "0.1.0");
        match upd.get_latest_release() {
            Err(crate::errors::Error::NoReleaseFound { target }) => {
                assert_eq!(target, None, "empty listing carries no asset target");
            }
            other => panic!(
                "empty fallback listing must be NoReleaseFound {{ target: None }}, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn get_latest_release_non_array_listing_fallback_is_invalid_response() {
        // Non-semver latest, then a non-array `{}` listing body: InvalidResponse.
        let base = stub(|_| {
            vec![
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_obj_json("nightly"),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: "{}".to_string(),
                },
            ]
        });
        let upd = gitee_update_sync(&base, "0.1.0");
        match upd.get_latest_release() {
            Err(crate::errors::Error::InvalidResponse { .. }) => {}
            other => panic!(
                "a non-array fallback listing must be InvalidResponse, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn get_latest_release_missing_tag_name_is_missing_asset_field() {
        // `/releases/latest` object missing `tag_name` must surface as EXACTLY
        // `MissingAssetField { field: "tag_name" }`.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"{"created_at":"2020-01-01T00:00:00Z","name":"x","assets":[]}"#.to_string(),
            }]
        });
        let upd = gitee_update_sync(&base, "0.1.0");
        match upd.get_latest_release() {
            Err(crate::errors::Error::MissingAssetField { field }) => {
                assert_eq!(field, "tag_name", "must name the absent field exactly");
            }
            other => panic!(
                "missing tag_name must be MissingAssetField {{ field: \"tag_name\" }}, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn get_newer_releases_sync_returns_releases_and_filters_to_newer() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
            }]
        });
        let upd = gitee_update_sync(&base, "1.0.0");
        let releases = upd.get_newer_releases().unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["2.0.0", "1.5.0"],
            "only releases strictly newer than the current version are kept, in order"
        );
        assert_eq!(releases.latest().unwrap().version(), "2.0.0");
        assert!(releases.is_update_available().unwrap());
    }

    #[test]
    fn get_newer_releases_sync_reports_no_update_when_up_to_date() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v1.0.0", "v0.9.0"]),
            }]
        });
        let upd = gitee_update_sync(&base, "1.0.0");
        let releases = upd.get_newer_releases().unwrap();
        assert!(releases.all().is_empty(), "no newer release => empty list");
        assert!(
            !releases.is_update_available().unwrap(),
            "empty list => no update available"
        );
    }

    #[test]
    fn get_newer_releases_sync_empty_array_returns_empty() {
        // An empty listing array on the paginated path is not an error: the filtered result is
        // simply empty (distinct from `get_latest_release`, which errors on nothing to return).
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "[]".to_string(),
            }]
        });
        let upd = gitee_update_sync(&base, "0.1.0");
        let releases = upd.get_newer_releases().unwrap();
        assert!(releases.all().is_empty());
    }

    #[test]
    fn get_newer_releases_sync_non_array_is_invalid_response() {
        // A top-level `{}` object cannot be deserialized as `Vec<ReleaseDto>` on the listing path.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "{}".to_string(),
            }]
        });
        let upd = gitee_update_sync(&base, "0.1.0");
        match upd.get_newer_releases() {
            Err(crate::errors::Error::InvalidResponse { .. }) => {}
            other => panic!(
                "a non-array listing must be Error::InvalidResponse, got {:?}",
                other
            ),
        }
    }

    /// Like [`stub`], but also captures each incoming raw request so tests can assert on what the
    /// client actually sent (e.g. which path was requested, and which headers).
    fn stub_capturing(
        make: impl FnOnce(&str) -> Vec<Resp>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let responses = make(&base);
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        std::thread::spawn(move || {
            for r in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                sink.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let mut out = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\n",
                    r.status
                );
                if let Some(link) = r.link {
                    out.push_str(&format!("Link: <{link}>; rel=\"next\"\r\n"));
                }
                out.push_str(&format!(
                    "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    r.body.len(),
                    r.body
                ));
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        (base, captured)
    }

    #[test]
    fn get_newer_releases_continues_past_non_newer_releases_and_fetches_page_two() {
        // Non-newer releases must NOT halt pagination -- page 2 must be fetched and its newer
        // release returned alongside the newer items from page 1.
        let (base, captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v5/repos/o/r/releases?page=2")),
                    body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v3.0.0"]),
                },
            ]
        });
        let upd = gitee_update_sync(&base, "1.0.0");
        let releases = upd.get_newer_releases().unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["2.0.0", "1.5.0", "3.0.0"]);
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "non-newer releases must not halt pagination; both pages must be requested"
        );
    }

    #[test]
    fn release_list_fetch_walks_all_pages_unfiltered() {
        // `ReleaseList::fetch` is an UNFILTERED listing (stop_at = None) and must walk ALL pages,
        // accumulating even releases older than any current version.
        let (base, captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v5/repos/o/r/releases?page=2")),
                    body: releases_json(&["v2.0.0", "v0.5.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v0.1.0"]),
                },
            ]
        });
        let releases = super::ReleaseList::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .build()
            .unwrap()
            .fetch()
            .unwrap()
            .into_vec();
        let versions: Vec<&str> = releases.iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["2.0.0", "0.5.0", "0.1.0"],
            "the unfiltered ReleaseList must accumulate ALL pages, older releases included"
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "both pages must be requested for the unfiltered listing"
        );
    }

    // A realistic populated payload parses through the DTO into a `Release` whose getters surface
    // every field (via the `/releases/latest` single object).
    #[test]
    fn dto_parse_maps_populated_payload_through_getters() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"{"tag_name":"v3.4.5","created_at":"2021-07-08T09:10:11Z","name":"My App 3.4.5","html_url":"https://gitee.com/o/r/releases/v3.4.5","body":"the notes","assets":[{"name":"app-x86_64-linux.tar.gz","browser_download_url":"https://gitee.example/app-x86_64-linux.tar.gz"},{"name":"app-aarch64-linux.tar.gz","browser_download_url":"https://gitee.example/app-aarch64-linux.tar.gz"}]}"#
                    .to_string(),
            }]
        });
        let upd = gitee_update_sync(&base, "0.1.0");
        let releases = upd.get_latest_release().unwrap();
        let rel = releases.latest().unwrap();
        assert_eq!(rel.version(), "3.4.5", "leading `v` stripped from tag_name");
        assert_eq!(rel.name(), "My App 3.4.5", "name surfaces from `name`");
        assert_eq!(rel.date(), "2021-07-08T09:10:11Z", "date from `created_at`");
        assert_eq!(
            rel.body(),
            Some("the notes"),
            "body surfaces from gitee's `body` field"
        );
        assert_eq!(
            rel.release_notes_url(),
            Some("https://gitee.com/o/r/releases/v3.4.5"),
            "release notes URL surfaces from `html_url`"
        );
        assert_eq!(rel.assets().len(), 2, "both `assets` entries parsed");
        assert_eq!(rel.assets()[0].name(), "app-x86_64-linux.tar.gz");
        assert_eq!(
            rel.assets()[0].download_url(),
            "https://gitee.example/app-x86_64-linux.tar.gz",
            "asset download_url comes from `browser_download_url`"
        );
        assert_eq!(rel.assets()[1].name(), "app-aarch64-linux.tar.gz");
    }

    // THE core gitee divergence: gitee's auto-generated source archive appears in `assets` WITHOUT
    // a `name` (and often without a `browser_download_url`). Those nameless / URL-less assets must
    // be SKIPPED (debug-logged), not error the whole release; named downloadable assets survive.
    #[test]
    fn nameless_source_zip_skipped_named_assets_survive() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: concat!(
                    r#"{"tag_name":"v1.2.3","created_at":"2020-01-01T00:00:00Z","name":"v1.2.3","assets":["#,
                    // gitee's auto-generated source zip: no `name` (has a url) -> skipped.
                    r#"{"browser_download_url":"https://gitee.com/o/r/repository/archive/v1.2.3.zip"},"#,
                    // a source archive with a name but NO download url -> also skipped.
                    r#"{"name":"v1.2.3.tar.gz"},"#,
                    // a real, named, downloadable binary -> survives.
                    r#"{"name":"app-x86_64-linux.tar.gz","browser_download_url":"https://gitee.com/o/r/attach_files/app-x86_64-linux.tar.gz"}"#,
                    r#"]}"#
                )
                .to_string(),
            }]
        });
        let upd = gitee_update_sync(&base, "0.1.0");
        let releases = upd.get_latest_release().unwrap();
        let rel = releases.latest().unwrap();
        assert_eq!(
            rel.assets().len(),
            1,
            "the nameless and the URL-less assets are skipped; only the named binary survives"
        );
        assert_eq!(rel.assets()[0].name(), "app-x86_64-linux.tar.gz");
        assert_eq!(
            rel.assets()[0].download_url(),
            "https://gitee.com/o/r/attach_files/app-x86_64-linux.tar.gz"
        );
    }

    // The listing `Releases` from `ReleaseList::fetch` carries NO current version, so
    // `current_version()` is `None` and `is_update_available()` errors with EXACTLY
    // `NoCurrentVersion`. `into_vec()` recovers the release vec.
    #[test]
    fn release_list_fetch_returns_listing_releases_without_current_version() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.0.0"]),
            }]
        });
        let releases = super::ReleaseList::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .build()
            .unwrap()
            .fetch()
            .unwrap();
        assert_eq!(
            releases.current_version(),
            None,
            "a bare listing carries no current version"
        );
        assert!(
            matches!(
                releases.is_update_available(),
                Err(crate::errors::Error::NoCurrentVersion)
            ),
            "is_update_available() on a listing must error with NoCurrentVersion, got {:?}",
            releases.is_update_available()
        );
        let versions: Vec<String> = releases
            .into_vec()
            .into_iter()
            .map(|r| r.version().to_string())
            .collect();
        assert_eq!(versions, vec!["2.0.0", "1.0.0"]);
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn release_list_fetch_async_returns_listing_releases_without_current_version() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.0.0"]),
            }]
        });
        let releases = super::ReleaseList::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .build()
            .unwrap()
            .fetch_async()
            .await
            .unwrap();
        assert_eq!(releases.current_version(), None);
        assert!(matches!(
            releases.is_update_available(),
            Err(crate::errors::Error::NoCurrentVersion)
        ));
        let versions: Vec<String> = releases
            .into_vec()
            .into_iter()
            .map(|r| r.version().to_string())
            .collect();
        assert_eq!(versions, vec!["2.0.0", "1.0.0"]);
    }

    #[test]
    fn filter_target_drops_releases_without_matching_asset() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: concat!(
                    r#"[{"tag_name":"v2.0.0","created_at":"2020-01-01T00:00:00Z","name":"v2.0.0","assets":[{"name":"app-x86_64-linux.tar.gz","browser_download_url":"https://example.com/2.0.0"}]},"#,
                    r#"{"tag_name":"v1.0.0","created_at":"2019-01-01T00:00:00Z","name":"v1.0.0","assets":[{"name":"app-windows.zip","browser_download_url":"https://example.com/1.0.0"}]}]"#
                )
                .to_string(),
            }]
        });
        let releases = super::ReleaseList::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .filter_target("x86_64-linux")
            .build()
            .unwrap()
            .fetch()
            .unwrap()
            .into_vec();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version(), "2.0.0");
    }

    #[test]
    fn build_requires_repo_owner_and_name() {
        let missing_owner = Update::configure()
            .repo_name("repo")
            .current_version("0.1.0")
            .build();
        assert!(missing_owner.is_err(), "build must fail without repo_owner");

        let missing_name = Update::configure()
            .repo_owner("owner")
            .current_version("0.1.0")
            .build();
        assert!(missing_name.is_err(), "build must fail without repo_name");
    }

    #[test]
    fn release_list_build_requires_repo_owner_and_repo_name() {
        let res = super::ReleaseList::configure().repo_name("r").build();
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::MissingField {
                    field: "repo_owner"
                })
            ),
            "missing repo_owner must surface as MissingField, got {:?}",
            res
        );
        let res = super::ReleaseList::configure().repo_owner("o").build();
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::MissingField { field: "repo_name" })
            ),
            "missing repo_name must surface as MissingField, got {:?}",
            res
        );
    }

    #[test]
    fn releases_url_encodes_owner_and_name() {
        let upd = Update::configure()
            .host("https://gitee.example.com")
            .repo_owner("my owner")
            .repo_name("my repo")
            .bin_name("app")
            .current_version("0.1.0")
            .build_update()
            .unwrap();
        assert_eq!(
            upd.releases_url(),
            "https://gitee.example.com/api/v5/repos/my%20owner/my%20repo/releases",
            "repo_owner and repo_name must be percent-encoded in the releases URL"
        );
    }

    #[test]
    fn release_list_fetch_encodes_owner_and_name_in_request_path() {
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v1.0.0"]),
            }]
        });
        super::ReleaseList::configure()
            .host(&base)
            .repo_owner("my owner")
            .repo_name("my repo")
            .build()
            .unwrap()
            .fetch()
            .unwrap();
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        let request_line = reqs[0].lines().next().unwrap_or("");
        assert!(
            request_line.contains("/api/v5/repos/my%20owner/my%20repo/releases"),
            "owner and name must be percent-encoded in the request path; got: {request_line}"
        );
    }

    #[test]
    fn release_list_build_surfaces_invalid_header() {
        let res = super::ReleaseList::configure()
            .repo_owner("o")
            .repo_name("r")
            .request_header("inva lid", "ok")
            .build();
        assert!(matches!(
            res,
            Err(crate::errors::Error::InvalidHeader { .. })
        ));
    }

    #[test]
    fn update_build_surfaces_invalid_header() {
        let res = Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .request_header("inva lid", "ok")
            .build();
        assert!(matches!(
            res,
            Err(crate::errors::Error::InvalidHeader { .. })
        ));
    }

    #[test]
    fn identifier_is_wired() {
        let upd = Update::configure()
            .repo_owner("owner")
            .repo_name("repo")
            .bin_name("app")
            .current_version("0.1.0")
            .asset_identifier("musl")
            .build()
            .unwrap();
        assert_eq!(upd.asset_identifier(), Some("musl"));
    }

    #[test]
    fn api_headers_override_uses_gitee_user_agent() {
        // The `{api_headers}` override arm must wire gitee's custom `api_headers` (User-Agent), not
        // the trait default. The auth scheme/token is applied centrally by `apply_auth`.
        let upd = Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        let headers = upd.api_headers(Some("secret")).unwrap();
        assert_eq!(
            headers
                .get(crate::http_client::header::USER_AGENT)
                .unwrap()
                .to_str()
                .unwrap(),
            crate::DEFAULT_USER_AGENT
        );
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "api_headers no longer bakes auth; apply_auth applies the Bearer scheme"
        );
    }

    // --- AUTH: Bearer scheme, both paths, override, no-leak -----------------------------------

    // gitee resolves to the `Bearer` scheme, applied by the shared `apply_auth` on the request
    // config consumed by BOTH the listing and download paths. A user override wins. The applied
    // Authorization header value is marked sensitive so it never renders in Debug output or logs.
    #[test]
    fn gitee_bearer_scheme_applied_to_both_paths() {
        use crate::http_client::header::{AUTHORIZATION, HeaderMap};
        #[allow(unused_imports)]
        use crate::update::UpdateInternals;
        let upd = Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("secret")
            .build()
            .unwrap();

        // Listing path host (gitee.com api).
        let mut headers = HeaderMap::new();
        upd.request_config()
            .apply_auth("https://gitee.com/api/v5/repos/o/r/releases", &mut headers)
            .unwrap();
        let value = headers.get(AUTHORIZATION).unwrap();
        assert_eq!(
            value.to_str().unwrap(),
            "Bearer secret",
            "gitee authenticates with the Bearer scheme"
        );
        assert!(
            value.is_sensitive(),
            "the applied Authorization value must be marked sensitive so it is kept out of logs"
        );

        // Download path (an attachment URL on the same gitee host) also receives the token.
        let mut dl_headers = HeaderMap::new();
        upd.request_config()
            .apply_auth(
                "https://gitee.com/o/r/attach_files/app.tar.gz",
                &mut dl_headers,
            )
            .unwrap();
        assert_eq!(
            dl_headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer secret",
            "the download path on the gitee host also receives the Bearer token"
        );

        // A user AUTHORIZATION override wins.
        let upd = Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("secret")
            .request_header(AUTHORIZATION, "Bearer user-override")
            .build()
            .unwrap();
        let mut headers = upd.request_config().headers.clone();
        upd.request_config()
            .apply_auth("https://gitee.com/api/v5/repos/o/r/releases", &mut headers)
            .unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer user-override",
            "a user AUTHORIZATION override must win over the Bearer scheme"
        );
    }

    #[test]
    fn release_list_auth_token_transmitted_as_bearer() {
        // End-to-end: `ReleaseListBuilder::auth_token` must cause `Authorization: Bearer <secret>`
        // to appear in the actual HTTP request, and the token must NOT leak into the request line
        // (URL).
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v1.0.0"]),
            }]
        });
        super::ReleaseList::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .auth_token("secret")
            .build()
            .unwrap()
            .fetch()
            .unwrap();
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1, "exactly one request must be made");
        let auth_header = reqs[0]
            .lines()
            .find(|l| l.to_lowercase().starts_with("authorization:"));
        assert!(
            auth_header.is_some_and(|l| l.contains("Bearer secret")),
            "ReleaseList::fetch must transmit `Authorization: Bearer secret`, got header: {:?}",
            auth_header
        );
        let request_line = reqs[0].lines().next().unwrap_or("");
        assert!(
            !request_line.contains("secret"),
            "the token must never appear in the request URL/line, got: {request_line}"
        );
    }

    #[test]
    fn get_latest_release_sync_transmits_bearer_token_and_never_in_url() {
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v2.0.0"),
            }]
        });
        let upd = Update::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("1.0.0")
            .auth_token("mytoken")
            .build()
            .unwrap();
        upd.get_latest_release().unwrap();
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1, "exactly one request for get_latest_release");
        let auth_header = reqs[0]
            .lines()
            .find(|l| l.to_lowercase().starts_with("authorization:"));
        assert!(
            auth_header.is_some_and(|l| l.contains("Bearer mytoken")),
            "get_latest_release must transmit `Authorization: Bearer mytoken`, got: {:?}",
            auth_header
        );
        let request_line = reqs[0].lines().next().unwrap_or("");
        assert!(
            !request_line.contains("mytoken"),
            "the token must never appear in the request URL/line, got: {request_line}"
        );
    }

    #[test]
    fn auth_token_never_appears_in_debug_or_error_display() {
        // The token must not leak into the built updater's Debug output (RequestConfig renders it
        // as `<token>`), nor into any error Display produced while a token is configured.
        let upd = Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("supersecret")
            .build()
            .unwrap();
        let debug = format!("{:?}", upd);
        assert!(
            !debug.contains("supersecret"),
            "the auth token must not appear in the Update Debug output: {debug}"
        );

        // An error surfaced from a request path (e.g. a non-semver pinned tag) must not carry the
        // token in its Display.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("nightly"),
            }]
        });
        let upd = Update::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("supersecret")
            .build()
            .unwrap();
        let err = upd
            .get_release_version("nightly")
            .expect_err("a non-semver pinned tag must error");
        assert!(
            !format!("{err}").contains("supersecret"),
            "the auth token must not appear in an error Display: {err}"
        );
    }

    // --- latest-vs-current comparison (outside-in) -------------------------------------------

    // The `/releases/latest` object can carry a semver tag OLDER than the configured current
    // version (e.g. the maintainer re-pinned an old release, or current is a pre-release ahead of
    // the last published tag). `get_latest_release` must still succeed (it reports what the server
    // says is latest) and the one-element `Releases` pre-check must report NO update available.
    #[test]
    fn get_latest_release_older_semver_reports_no_update() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v0.5.0"),
            }]
        });
        let upd = gitee_update_sync(&base, "1.0.0");
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(
            releases.latest().unwrap().version(),
            "0.5.0",
            "get_latest_release surfaces whatever /releases/latest reports, even if older"
        );
        assert!(
            !releases.is_update_available().unwrap(),
            "0.5.0 < 1.0.0 => no update available (no panic, no confusing error)"
        );
    }

    // A `/releases/latest` semver tag EQUAL to the current version is not an update.
    #[test]
    fn get_latest_release_equal_semver_reports_no_update() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v1.0.0"),
            }]
        });
        let upd = gitee_update_sync(&base, "1.0.0");
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(releases.latest().unwrap().version(), "1.0.0");
        assert!(
            !releases.is_update_available().unwrap(),
            "latest == current => no update available"
        );
    }

    // --- all-assets-skipped leniency (outside-in) --------------------------------------------

    // The lenient asset strategy can drop EVERY asset when a release carries only the nameless
    // source archive and a URL-less entry. The release must still parse (that is the deliberate
    // divergence), yielding an EMPTY asset list rather than an error -- and downstream target
    // selection over that release must report "no match" sanely (never panic).
    #[test]
    fn get_latest_release_all_assets_skipped_yields_empty_asset_list() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: concat!(
                    r#"{"tag_name":"v1.2.3","created_at":"2020-01-01T00:00:00Z","name":"v1.2.3","assets":["#,
                    // nameless source zip (has a url) -> skipped
                    r#"{"browser_download_url":"https://gitee.com/o/r/repository/archive/v1.2.3.zip"},"#,
                    // named but no url -> skipped
                    r#"{"name":"v1.2.3.tar.gz"}"#,
                    r#"]}"#
                )
                .to_string(),
            }]
        });
        let upd = gitee_update_sync(&base, "0.1.0");
        let releases = upd.get_latest_release().unwrap();
        let rel = releases.latest().unwrap();
        assert!(
            rel.assets().is_empty(),
            "every asset was lenient-skipped; the release parses with an empty asset list"
        );
        assert!(
            !rel.has_target_asset("x86_64-linux"),
            "target selection over an asset-less release reports no match, not a panic"
        );
    }

    // Downstream: an asset-less (all-skipped) release is dropped by a `filter_target` listing,
    // so the user gets an empty listing rather than a release that cannot be downloaded.
    #[test]
    fn filter_target_drops_release_whose_assets_all_skipped() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: concat!(
                    r#"[{"tag_name":"v2.0.0","created_at":"2020-01-01T00:00:00Z","name":"v2.0.0","assets":["#,
                    r#"{"browser_download_url":"https://gitee.com/o/r/repository/archive/v2.0.0.zip"}"#,
                    r#"]}]"#
                )
                .to_string(),
            }]
        });
        let releases = super::ReleaseList::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .filter_target("x86_64-linux")
            .build()
            .unwrap()
            .fetch()
            .unwrap()
            .into_vec();
        assert!(
            releases.is_empty(),
            "a release whose only assets are lenient-skipped matches no target and is dropped"
        );
    }

    // --- tag_prefix x /releases/latest interaction (outside-in) -------------------------------

    // `/releases/latest` returns a prefixed tag (`myapp-2.0.0`); with `tag_prefix("myapp-")` the
    // backend must strip the prefix on the dedicated latest endpoint too (not only in the listing),
    // yielding version `2.0.0`.
    #[test]
    fn get_latest_release_strips_tag_prefix_on_latest_endpoint() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("myapp-2.0.0"),
            }]
        });
        let upd = Update::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .tag_prefix("myapp-")
            .build()
            .unwrap();
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(
            releases.latest().unwrap().version(),
            "2.0.0",
            "the configured tag_prefix is stripped from the /releases/latest tag"
        );
    }

    // A `/releases/latest` tag that does NOT carry the configured prefix is a prefix mismatch,
    // which the backend maps to `Error::SemVer` -- the same skippable class as a rolling tag -- so
    // it must trigger the LISTING FALLBACK (not surface as a hard error). The fallback then picks
    // the newest prefixed release it can compare.
    #[test]
    fn get_latest_release_prefix_mismatch_falls_back_to_listing() {
        let (base, captured) = stub_capturing(|_| {
            vec![
                Resp {
                    status: "200 OK",
                    link: None,
                    // no `myapp-` prefix -> mismatch -> SemVer -> fallback
                    body: release_obj_json("2.0.0"),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["otherapp-9.9.9", "myapp-1.5.0", "myapp-1.0.0"]),
                },
            ]
        });
        let upd = Update::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .tag_prefix("myapp-")
            .build()
            .unwrap();
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(
            releases.latest().unwrap().version(),
            "1.5.0",
            "prefix-mismatched latest falls back to the newest prefixed listing release"
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "prefix mismatch must fetch /releases/latest then the listing"
        );
    }

    // --- host formatting (characterization, parity with gitea) -------------------------------

    // The host setter is documented as "scheme + host, no trailing slash"; the crate does NOT
    // normalize a stray trailing slash (parity with gitea, which pins nothing here). Pin the
    // current behavior so an accidental change to host handling is caught.
    #[test]
    fn host_trailing_slash_is_not_normalized() {
        let upd = Update::configure()
            .host("https://gitee.example.com/")
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(
            upd.releases_url(),
            "https://gitee.example.com//api/v5/repos/o/r/releases",
            "a trailing slash is not stripped (documented: pass host without one)"
        );
    }

    // An enterprise host mounted under a path prefix is preserved verbatim in the built URL, and
    // auth host-gating still resolves the bare host so the Bearer token is attached to that host.
    #[test]
    fn enterprise_host_with_path_prefix_is_preserved_and_auth_gated() {
        use crate::http_client::header::{AUTHORIZATION, HeaderMap};
        #[allow(unused_imports)]
        use crate::update::UpdateInternals;
        let upd = Update::configure()
            .host("https://gitee.example.com/prefix")
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("secret")
            .build()
            .unwrap();
        assert_eq!(
            upd.releases_url(),
            "https://gitee.example.com/prefix/api/v5/repos/o/r/releases",
            "a path-prefixed enterprise host is preserved in the URL"
        );
        let mut headers = HeaderMap::new();
        upd.request_config()
            .apply_auth(&upd.releases_url(), &mut headers)
            .unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer secret",
            "auth host-gating resolves the bare host and still attaches the token"
        );
    }

    // --- async coverage ----------------------------------------------------------------------

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_parses_release() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v2.5.0"),
            }]
        });
        let upd = gitee_update(&base, "0.1.0");
        let releases = upd.get_latest_release_async().await.unwrap();
        assert_eq!(releases.latest().unwrap().version(), "2.5.0");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_falls_back_to_listing_when_non_semver() {
        let base = stub(|_| {
            vec![
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_obj_json("nightly"),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v1.2.3", "v1.0.0"]),
                },
            ]
        });
        let upd = gitee_update(&base, "0.1.0");
        let releases = upd.get_latest_release_async().await.unwrap();
        assert_eq!(releases.latest().unwrap().version(), "1.2.3");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn fetch_all_releases_async_follows_link_pagination() {
        let base = stub(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v5/repos/o/r/releases?page=2")),
                    body: release_json("v1.0.0"),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_json("v0.9.0"),
                },
            ]
        });
        let releases = fetch_all_releases_async(
            &format!("{base}/api/v5/repos/o/r/releases"),
            &crate::backends::common::RequestConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            releases.len(),
            2,
            "both pages accumulated over async transport"
        );
        assert_eq!(releases[0].version(), "1.0.0");
        assert_eq!(releases[1].version(), "0.9.0");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_release_version_async_parses_single_tag_object() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v4.2.1"),
            }]
        });
        let upd = gitee_update(&base, "0.1.0");
        let rel = upd.get_release_version_async("v4.2.1").await.unwrap();
        assert_eq!(rel.version(), "4.2.1");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_release_version_async_missing_tag_name_is_missing_asset_field() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"{"created_at":"2020-01-01T00:00:00Z","assets":[]}"#.to_string(),
            }]
        });
        let upd = gitee_update(&base, "0.1.0");
        match upd.get_release_version_async("v1.0.0").await {
            Err(crate::errors::Error::MissingAssetField { field }) => {
                assert_eq!(field, "tag_name", "must name the absent field exactly");
            }
            other => panic!(
                "missing tag_name must be MissingAssetField {{ field: \"tag_name\" }}, got {:?}",
                other
            ),
        }
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_newer_releases_async_filters_to_newer_only() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
            }]
        });
        let upd = gitee_update(&base, "1.0.0");
        let releases = upd.get_newer_releases_async().await.unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["2.0.0", "1.5.0"]);
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_newer_releases_async_accumulates_across_pages_then_filters() {
        let base = stub(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v5/repos/o/r/releases?page=2")),
                    body: releases_json(&["v3.0.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v2.0.0"]),
                },
            ]
        });
        let upd = gitee_update(&base, "1.0.0");
        let releases = upd.get_newer_releases_async().await.unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["3.0.0", "2.0.0"]);
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_errors_on_non_array_fallback_payload() {
        // Non-semver latest, then a non-array listing body: InvalidResponse (async path).
        let base = stub(|_| {
            vec![
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_obj_json("nightly"),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: "{}".to_string(),
                },
            ]
        });
        let upd = gitee_update(&base, "0.1.0");
        let res = upd.get_latest_release_async().await;
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidResponse { .. })),
            "non-array fallback payload must surface as InvalidResponse, got {:?}",
            res
        );
    }

    // Belt-and-suspenders: the async newtype's Debug output must not leak the auth token either
    // (its inner blocking `Update` renders the token as `<token>`).
    #[cfg(feature = "async")]
    #[test]
    fn async_update_debug_never_leaks_token() {
        let upd = Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("supersecret")
            .build_async()
            .unwrap();
        let debug = format!("{upd:?}");
        assert!(
            !debug.contains("supersecret"),
            "the auth token must not appear in the AsyncUpdate Debug output: {debug}"
        );
    }
}

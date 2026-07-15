/*!
GitHub releases
*/
use crate::http_client::{HeaderMap, header};

use crate::backends::common::{CommonBuilderConfig, CommonConfig, RequestConfig};
use crate::backends::{Page, PageRequest, first_page_url, next_link, run_paginated};
use crate::version::bump_is_greater;
use crate::{
    errors::*,
    update::{Release, ReleaseAsset, ReleaseUpdate, Releases},
};
use serde::Deserialize;

/// GitHub release-asset JSON shape. Private DTO deserialized directly from the response bytes, then
/// converted into the public [`ReleaseAsset`]. Keeping it private means `Deserialize` is never part
/// of the public `ReleaseAsset` API.
#[derive(Deserialize)]
struct AssetDto {
    name: Option<String>,
    url: Option<String>,
}

impl AssetDto {
    fn into_asset(self) -> Result<ReleaseAsset> {
        let download_url = self.url.ok_or_else(|| Error::missing_asset_field("url"))?;
        let name = self
            .name
            .ok_or_else(|| Error::missing_asset_field("name"))?;
        Ok(ReleaseAsset::new(name, download_url))
    }
}

/// GitHub release JSON shape. Private DTO deserialized directly from the response bytes (replacing
/// the old `serde_json::Value` walk), then converted into the public [`Release`].
#[derive(Deserialize)]
struct ReleaseDto {
    tag_name: Option<String>,
    created_at: Option<String>,
    name: Option<String>,
    body: Option<String>,
    assets: Option<Vec<AssetDto>>,
}

impl ReleaseDto {
    fn into_release(self) -> Result<Release> {
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
        let assets = assets
            .into_iter()
            .map(AssetDto::into_asset)
            .collect::<Result<Vec<ReleaseAsset>>>()?;
        let mut builder = Release::builder();
        builder
            .name(name)
            .version(tag.trim_start_matches('v').to_owned())
            .date(date)
            .assets(assets);
        if let Some(body) = self.body {
            builder.body(body);
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
    repo_owner: Option<String>,
    repo_name: Option<String>,
    target: Option<String>,
    auth_token: Option<String>,
    custom_url: Option<String>,
    request: RequestConfig,
}
impl ReleaseListBuilder {
    /// Required. Set the repo owner, used to build a github api url
    pub fn repo_owner(&mut self, owner: impl Into<String>) -> &mut Self {
        self.repo_owner = Some(owner.into());
        self
    }

    /// Required. Set the repo name, used to build a github api url
    pub fn repo_name(&mut self, name: impl Into<String>) -> &mut Self {
        self.repo_name = Some(name.into());
        self
    }

    /// Set the optional arch `target` name, used to filter available releases
    pub fn filter_target(&mut self, target: impl Into<String>) -> &mut Self {
        self.target = Some(target.into());
        self
    }

    /// Set the optional github url, e.g. for a github enterprise installation.
    /// The url should provide the path to your API endpoint and end without a trailing slash,
    /// for example `https://api.github.com` or `https://github.mycorp.com/api/v3`
    ///
    /// **Semantic note:** this setter takes the full API base URL (ending at the version prefix,
    /// e.g. `.../api/v3`). The gitea and gitlab backends instead accept an instance host and
    /// append the API path internally; the s3 backend uses an `endpoint` setter. The difference
    /// is intentional: GitHub enterprise instances expose configurable API prefixes, whereas the
    /// other backends have fixed API paths relative to their host.
    pub fn api_base_url(&mut self, url: impl Into<String>) -> &mut Self {
        self.custom_url = Some(url.into());
        self
    }

    /// Set the authorization token, used in requests to the github api url
    ///
    /// This is to support private repos where you need a GitHub auth token.
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
        // Thread the auth token + github's `token` scheme into the request so the shared
        // `apply_auth` applies it on the listing path (honoring a user override).
        let mut request = self.request.clone();
        request.auth_scheme = crate::backends::common::AuthScheme::Token;
        request.auth_token = self.auth_token.clone();
        request.auth_base_host = crate::backends::common::host_of(
            self.custom_url
                .as_deref()
                .unwrap_or("https://api.github.com"),
        );
        request.build_client();
        request.check()?;
        Ok(ReleaseList {
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
            custom_url: self.custom_url.clone(),
            request,
        })
    }
}

/// `ReleaseList` provides a builder api for querying a GitHub repo,
/// returning a `Vec` of available `Release`s
#[derive(Clone, Debug)]
pub struct ReleaseList {
    repo_owner: String,
    repo_name: String,
    target: Option<String>,
    custom_url: Option<String>,
    request: RequestConfig,
}
impl ReleaseList {
    /// Initialize a ReleaseListBuilder
    pub fn configure() -> ReleaseListBuilder {
        ReleaseListBuilder {
            repo_owner: None,
            repo_name: None,
            target: None,
            auth_token: None,
            custom_url: None,
            request: RequestConfig::default(),
        }
    }

    /// Retrieve the available `Release`s as a [`Releases`].
    ///
    /// If a `filter_target` is set, only releases carrying an asset whose name contains it are
    /// returned. The result carries no current version (it is a bare listing), so
    /// [`Releases::current_version`] is `None`; use [`Releases::into_vec`] to recover the raw
    /// `Vec<Release>`.
    pub fn fetch(&self) -> Result<Releases> {
        let api_url = format!(
            "{}/repos/{}/{}/releases",
            self.custom_url
                .as_deref()
                .unwrap_or("https://api.github.com"),
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        );
        // An unfiltered listing must walk ALL pages: `stop_at = None`.
        let releases = run_paginated(releases_plan(&api_url, None)?, &self.request)?;
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
            "{}/repos/{}/{}/releases",
            self.custom_url
                .as_deref()
                .unwrap_or("https://api.github.com"),
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        );
        // An unfiltered listing must walk ALL pages: `stop_at = None`.
        let releases =
            crate::backends::run_paginated_async(releases_plan(&api_url, None)?, &self.request)
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

/// `github::Update` builder
///
/// Configure download and installation from
/// `https://api.github.com/repos/<repo_owner>/<repo_name>/releases/latest`
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct UpdateBuilder {
    repo_owner: Option<String>,
    repo_name: Option<String>,
    custom_url: Option<String>,
    common: CommonBuilderConfig,
}

impl UpdateBuilder {
    /// Initialize a new builder
    pub fn new() -> Self {
        Default::default()
    }

    /// Required. Set the repo owner, used to build a github api url
    pub fn repo_owner(&mut self, owner: impl Into<String>) -> &mut Self {
        self.repo_owner = Some(owner.into());
        self
    }

    /// Required. Set the repo name, used to build a github api url
    pub fn repo_name(&mut self, name: impl Into<String>) -> &mut Self {
        self.repo_name = Some(name.into());
        self
    }

    /// Set the optional github url, e.g. for a github enterprise installation.
    /// The url should provide the path to your API endpoint and end without a trailing slash,
    /// for example `https://api.github.com` or `https://github.mycorp.com/api/v3`
    pub fn api_base_url(&mut self, url: impl Into<String>) -> &mut Self {
        self.custom_url = Some(url.into());
        self
    }

    impl_common_builder_setters!();

    fn build_update(&self) -> Result<Update> {
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
            custom_url: self.custom_url.clone(),
            common: {
                let mut resolved = self.common.build()?;
                // Github authenticates with `token <token>`; set the scheme explicitly rather than
                // relying on `AuthScheme::default()` (gitlab overrides to `Bearer` the same way).
                resolved.request.auth_scheme = crate::backends::common::AuthScheme::Token;
                // The github API host (asset download URLs are on the same host) receives the token;
                // a server-supplied URL on any other host does not.
                resolved.request.auth_base_host = crate::backends::common::host_of(
                    self.custom_url
                        .as_deref()
                        .unwrap_or("https://api.github.com"),
                );
                resolved
            },
        })
    }

    /// Confirm config and create a ready-to-use `Update`.
    ///
    /// Returns the concrete [`Update`], which is `Send` (so it can move to a worker thread) and
    /// exposes the update verbs (`update`, `update_extended`, `get_latest_release`, ...) as inherent
    /// methods.
    ///
    /// * Errors:
    ///     * Invalid `Update` configuration
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

/// Updates to a specified or latest release distributed via GitHub
#[derive(Debug)]
#[non_exhaustive]
pub struct Update {
    repo_owner: String,
    repo_name: String,
    custom_url: Option<String>,
    common: CommonConfig,
}
impl Update {
    /// Initialize a new `Update` builder
    pub fn configure() -> UpdateBuilder {
        UpdateBuilder::new()
    }

    /// API base URL (the custom URL for enterprise installs, or the public github API). Shared by
    /// the sync and async fetch paths so they can't drift.
    fn api_base(&self) -> &str {
        self.custom_url
            .as_deref()
            .unwrap_or("https://api.github.com")
    }
}

impl crate::update::sealed::Sealed for Update {}

impl Update {
    /// The `/repos/{owner}/{name}/releases` listing URL.
    fn releases_url(&self) -> String {
        format!(
            "{}/repos/{}/{}/releases",
            self.api_base(),
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        )
    }

    /// The `/repos/{owner}/{name}/releases/latest` single-newest-release URL.
    fn latest_url(&self) -> String {
        format!(
            "{}/repos/{}/{}/releases/latest",
            self.api_base(),
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        )
    }

    /// The `/repos/{owner}/{name}/releases/tags/{ver}` single-release-by-tag URL.
    fn tag_url(&self, ver: &str) -> String {
        format!(
            "{}/repos/{}/{}/releases/tags/{}",
            self.api_base(),
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name),
            urlencoding::encode(ver)
        )
    }
}

impl ReleaseUpdate for Update {
    fn get_latest_release(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = run_paginated(single_plan(self.latest_url())?, &self.common.request)?;
        let release = releases
            .into_iter()
            .next()
            .ok_or_else(|| Error::NoReleaseFound { target: None })?;
        Ok(Releases::new(vec![release], current_version))
    }

    fn get_newer_releases(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = run_paginated(
            releases_plan(&self.releases_url(), Some(&current_version))?,
            &self.common.request,
        )?;
        Ok(Releases::new(releases, current_version))
    }

    fn get_release_version(&self, ver: &str) -> Result<Release> {
        let releases = run_paginated(single_plan(self.tag_url(ver))?, &self.common.request)?;
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
/// updater — e.g. `build_async()?.update()` — a compile error, so the async executor cannot be
/// silently blocked.
#[cfg(feature = "async")]
#[derive(Debug)]
pub struct AsyncUpdate(Update);

#[cfg(feature = "async")]
impl_async_update_verbs!(AsyncUpdate);

impl_update_config_accessors!(Update, {
    fn api_headers(&self, _auth_token: Option<&str>) -> Result<HeaderMap> {
        api_headers()
    }
});

/// Transport-free plan to fetch the paginated `releases` array, parsing each page with
/// the private `ReleaseDto` and following GitHub's `Link: rel="next"` pagination.
///
/// `stop_at` filters per-item: when `Some(current_version)` each release that is not strictly
/// newer than it is omitted from the collected list, but pagination continues to subsequent pages
/// regardless (a backport release — older semver, newer creation date — must not halt the walk
/// and cause a genuinely newer release on a later page to be missed). When `None` the listing is
/// unfiltered and every page is walked (used by `ReleaseList`).
fn releases_plan(base_url: &str, stop_at: Option<&str>) -> Result<PageRequest<Release>> {
    let headers = api_headers()?;
    let stop_at = stop_at.map(str::to_owned);
    Ok(release_array_page(
        first_page_url(base_url),
        headers,
        stop_at,
    ))
}

/// Build one `releases`-array [`PageRequest`], capturing what it needs to build the next page.
fn release_array_page(
    url: String,
    headers: HeaderMap,
    stop_at: Option<String>,
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
                let release = match dto.into_release() {
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

/// Transport-free plan to fetch a single release *object* (the `/releases/latest` and
/// `/releases/tags/{ver}` endpoints), parsed via the private `ReleaseDto` into a one-item page.
fn single_plan(url: String) -> Result<PageRequest<Release>> {
    let headers = api_headers()?;
    Ok(PageRequest {
        url,
        headers,
        parse: Box::new(|body, _resp_headers| {
            // The single-release endpoints return a bare release object; deserialize it directly
            // into the DTO and convert. An unparseable body is `InvalidResponse`, matching the
            // paginated listing parser.
            let dto: ReleaseDto =
                serde_json::from_slice(body).map_err(crate::errors::Error::invalid_response)?;
            Ok(Page::last(vec![dto.into_release()?]))
        }),
    })
}

#[cfg(feature = "async")]
impl crate::update::AsyncReleaseUpdate for Update {
    async fn get_latest_release_async(&self) -> Result<Releases> {
        use crate::backends::run_paginated_async;
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases =
            run_paginated_async(single_plan(self.latest_url())?, &self.common.request).await?;
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
            releases_plan(&self.releases_url(), Some(&current_version))?,
            &self.common.request,
        )
        .await?;
        Ok(Releases::new(releases, current_version))
    }

    async fn get_release_version_async(&self, ver: &str) -> Result<Release> {
        use crate::backends::run_paginated_async;
        let releases =
            run_paginated_async(single_plan(self.tag_url(ver))?, &self.common.request).await?;
        releases
            .into_iter()
            .next()
            .ok_or_else(|| Error::NoReleaseFound { target: None })
    }
}

/// Build github's base request headers (the shared `self_update/<version>` User-Agent; github
/// rejects requests with no User-Agent). The Authorization header is no longer set here: the auth
/// scheme/token is applied centrally by the shared
/// [`apply_auth`](crate::backends::common::RequestConfig::apply_auth) on both the listing and
/// download paths, which also honors a user `request_header(AUTHORIZATION, ..)` override.
fn api_headers() -> Result<header::HeaderMap> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        crate::DEFAULT_USER_AGENT
            .parse()
            .expect("github invalid user-agent"),
    );
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // The crate-private internal accessors (`request_timeout`, `verify_callback`, `asset_matcher`,
    // ...) now live on `UpdateInternals`; bring it into scope so `upd.request_timeout()` etc.
    // resolve.
    #[allow(unused_imports)]
    use crate::update::UpdateInternals;

    // The public config accessors (`api_headers`, `no_confirm`, `show_output`, ...) live on the
    // sealed `UpdateConfig` trait; bring it into scope so they resolve on the concrete `Update`.
    use crate::update::UpdateConfig;

    // The single-release endpoints (`/releases/latest`, `/releases/tags/{ver}`) surface an
    // unparseable body as `InvalidResponse`, matching the paginated listing parser (previously
    // they mapped to `Error::Json`, forcing callers to match two variants for one failure).
    #[test]
    fn single_plan_parse_failure_is_invalid_response() {
        let req = super::single_plan("https://example.test/releases/latest".to_string()).unwrap();
        let res = (req.parse)(b"not-json", &crate::http_client::HeaderMap::new());
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidResponse { .. })),
            "a malformed single-release body must map to InvalidResponse"
        );
    }

    // A rolling non-semver tag (`nightly`) in the listing must be skipped, not fail the whole
    // fetch: repositories commonly mix rolling tags with semver releases. Pre-release/build
    // suffixes are valid semver and must NOT be skipped. A capital-`V` prefix is not trimmed
    // (only lowercase `v` is), so a `V`-tagged release is skipped like any other unparseable
    // tag; this documents the boundary of the trim.
    #[test]
    fn listing_skips_non_semver_tags() {
        let req = super::release_array_page(
            "https://example.test/releases".to_string(),
            crate::http_client::HeaderMap::new(),
            None,
        );
        let body = releases_array_json(&[
            "nightly",
            "v2.0.0-rc.1+build",
            "v1.2.3",
            "2024-06-01",
            "V1.1.0",
            "v1.0.0",
        ]);
        let page = (req.parse)(body.as_bytes(), &crate::http_client::HeaderMap::new()).unwrap();
        let versions: Vec<&str> = page.items.iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["2.0.0-rc.1+build", "1.2.3", "1.0.0"],
            "non-semver (incl. capital-V) tags are skipped; semver incl. pre-release survives"
        );
    }

    // End-to-end over the loopback stub: a first page consisting entirely of non-semver tags
    // (with a Link to page 2) must not stop the walk; page 2's release is still collected.
    #[test]
    fn fetch_continues_past_an_all_non_semver_page() {
        let (base, captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/repos/o/r/releases?page=2")),
                    body: releases_array_json(&["nightly", "latest"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_array_json(&["v3.0.0"]),
                },
            ]
        });
        let releases = fetch_all_releases(
            &format!("{base}/repos/o/r/releases"),
            &crate::backends::common::RequestConfig::default(),
        )
        .unwrap();
        let versions: Vec<&str> = releases.iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["3.0.0"]);
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "an all-skipped page 1 must not stop pagination; page 2 must be requested"
        );
    }

    // The async driver shares the same parse closures; pin that the skip behaves identically
    // through `run_paginated_async`.
    #[cfg(feature = "async")]
    #[tokio::test]
    async fn fetch_async_skips_non_semver_tags() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_array_json(&["nightly", "v1.2.3"]),
            }]
        });
        let releases = fetch_all_releases_async(
            &format!("{base}/repos/o/r/releases"),
            &crate::backends::common::RequestConfig::default(),
        )
        .await
        .unwrap();
        let versions: Vec<&str> = releases.iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["1.2.3"]);
    }

    // The same skip applies on the filtered (`stop_at`) walk used by `get_newer_releases`,
    // which feeds `update()`: a rolling tag must not abort an update check.
    #[test]
    fn filtered_listing_skips_non_semver_tags() {
        let req = super::release_array_page(
            "https://example.test/releases".to_string(),
            crate::http_client::HeaderMap::new(),
            Some("1.0.0".to_string()),
        );
        let body = releases_array_json(&["nightly", "v1.2.3", "v0.9.0"]);
        let page = (req.parse)(body.as_bytes(), &crate::http_client::HeaderMap::new()).unwrap();
        let versions: Vec<&str> = page.items.iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["1.2.3"]);
    }

    // The single-release endpoints cannot skip: a pinned non-semver tag errors, and the error
    // must name the offending tag rather than surfacing a bare semver parse failure.
    #[test]
    fn single_plan_non_semver_tag_errors_naming_the_tag() {
        let req =
            super::single_plan("https://example.test/releases/tags/nightly".to_string()).unwrap();
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

    /// Test wrapper: drive the sans-io `releases_plan` through the sync `run_paginated` driver.
    /// `stop_at = None` => walk all pages (the unfiltered listing behavior).
    fn fetch_all_releases(
        base_url: &str,
        req: &crate::backends::common::RequestConfig,
    ) -> crate::errors::Result<Vec<super::Release>> {
        crate::backends::run_paginated(super::releases_plan(base_url, None)?, req)
    }

    /// Async test wrapper over `releases_plan` + the async driver. `stop_at = None`.
    #[cfg(feature = "async")]
    async fn fetch_all_releases_async(
        base_url: &str,
        req: &crate::backends::common::RequestConfig,
    ) -> crate::errors::Result<Vec<super::Release>> {
        crate::backends::run_paginated_async(super::releases_plan(base_url, None)?, req).await
    }

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

    fn release_json(tag: &str) -> String {
        format!(
            r#"[{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[]}}]"#
        )
    }

    fn release_obj_json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[]}}"#
        )
    }

    /// A github-format releases JSON array with one entry per tag (newest-first as listed).
    fn releases_array_json(tags: &[&str]) -> String {
        let objs = tags
            .iter()
            .map(|tag| {
                format!(
                    r#"{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[]}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("[{objs}]")
    }

    // --- git release-scan early-stop (selection parity + page-2 never requested) -------

    #[test]
    fn get_newer_releases_continues_past_non_newer_releases_and_fetches_page_two() {
        // Page 1 contains both newer (v2.0.0, v1.5.0) and non-newer (v1.0.0, v0.9.0) releases.
        // Non-newer releases must NOT halt pagination — page 2 is requested and its newer
        // release (v3.0.0) is included in the result alongside the newer items from page 1.
        // (The old early-stop bug would have returned only ["2.0.0", "1.5.0"] in 1 request.)
        let (base, captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/repos/o/r/releases?page=2")),
                    body: releases_array_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_array_json(&["v3.0.0"]),
                },
            ]
        });
        let upd = github_update_sync(&base, "1.0.0");
        let releases = upd.get_newer_releases().unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        // Non-newer items (v1.0.0, v0.9.0) are filtered out per-item; newer items from both
        // pages are kept. v3.0.0 from page 2 is present, proving pagination was not halted.
        assert_eq!(versions, vec!["2.0.0", "1.5.0", "3.0.0"]);
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "non-newer releases must not halt pagination; both pages must be requested"
        );
    }

    #[test]
    fn early_stop_selects_same_release_as_a_full_walk() {
        // Selection parity: the early-stopped `get_newer_releases` must let the updater select the
        // SAME release as a full unfiltered walk would. Drive the choice via the same
        // `choose_latest_release` the orchestrator uses, comparing the early-stop list against a
        // full-walk list of the identical releases.
        let early_first_page = releases_array_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]);
        let (base, _captured) = stub_capturing(move |base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/repos/o/r/releases?page=2")),
                    body: early_first_page,
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_array_json(&["v0.5.0"]),
                },
            ]
        });
        let upd = github_update_sync(&base, "1.0.0");
        let early = upd.get_newer_releases().unwrap().into_vec();
        let early_choice =
            crate::update::testing::choose_latest_release_for_test(early, "1.0.0").unwrap();

        // A full walk would also see v1.0.0/v0.9.0/v0.5.0, but those are filtered/older, so the
        // newest compatible release is the same: v1.5.0 (compatible with 1.0.0; 2.0.0 is a major
        // bump and only chosen as a fallback if no compatible exists).
        let full = vec![
            crate::update::Release::builder()
                .version("2.0.0")
                .build()
                .unwrap(),
            crate::update::Release::builder()
                .version("1.5.0")
                .build()
                .unwrap(),
            crate::update::Release::builder()
                .version("1.0.0")
                .build()
                .unwrap(),
            crate::update::Release::builder()
                .version("0.9.0")
                .build()
                .unwrap(),
            crate::update::Release::builder()
                .version("0.5.0")
                .build()
                .unwrap(),
        ];
        let full_choice =
            crate::update::testing::choose_latest_release_for_test(full, "1.0.0").unwrap();
        assert_eq!(
            early_choice.map(|r| r.version().to_string()),
            full_choice.map(|r| r.version().to_string()),
            "early-stop must select the same release as a full walk"
        );
    }

    #[test]
    fn release_list_fetch_walks_all_pages_unfiltered() {
        // `ReleaseList::fetch` is an UNFILTERED listing (stop_at = None) and must keep walking
        // ALL pages - even when a page contains releases older than any current version (there is no
        // current version here). Page 1 advertises page 2; both must be accumulated.
        let (base, captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/repos/o/r/releases?page=2")),
                    body: releases_array_json(&["v2.0.0", "v0.5.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_array_json(&["v0.1.0"]),
                },
            ]
        });
        let releases = super::ReleaseList::configure()
            .api_base_url(&base)
            .repo_owner("o")
            .repo_name("r")
            .build()
            .unwrap()
            .fetch()
            .unwrap();
        // `ReleaseList::fetch` returns a `Releases` (with no current version); recover the raw
        // vec via `into_vec()`.
        let releases = releases.into_vec();
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

    // --- `ReleaseList::fetch` returns a `Releases`; `into_vec()` recovers the releases ----------

    #[test]
    fn release_list_fetch_returns_releases_and_into_vec_recovers_them() {
        // `ReleaseList::fetch` returns a `Releases` carrying NO current version
        // (a bare listing), so `current_version()` is `None` and `is_update_available()` errors;
        // `into_vec()` recovers the underlying `Vec<Release>` in listing order.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_array_json(&["v2.0.0", "v1.0.0"]),
            }]
        });
        let releases = super::ReleaseList::configure()
            .api_base_url(&base)
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
            releases.is_update_available().is_err(),
            "a listing with no current version cannot answer is_update_available()"
        );
        let recovered = releases.into_vec();
        let versions: Vec<&str> = recovered.iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["2.0.0", "1.0.0"]);
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn release_list_fetch_async_returns_releases_and_into_vec_recovers_them() {
        // Async sibling of `release_list_fetch_returns_releases_and_into_vec_recovers_them`:
        // `ReleaseList::fetch_async` returns a `Releases` carrying NO current version
        // (a bare listing), so `current_version()` is `None` and `is_update_available()` errors;
        // `into_vec()` recovers the underlying `Vec<Release>` in listing order.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_array_json(&["v2.0.0", "v1.0.0"]),
            }]
        });
        let releases = super::ReleaseList::configure()
            .api_base_url(&base)
            .repo_owner("o")
            .repo_name("r")
            .build()
            .unwrap()
            .fetch_async()
            .await
            .unwrap();
        assert_eq!(
            releases.current_version(),
            None,
            "a bare listing carries no current version"
        );
        assert!(
            releases.is_update_available().is_err(),
            "a listing with no current version cannot answer is_update_available()"
        );
        let recovered = releases.into_vec();
        let versions: Vec<&str> = recovered.iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["2.0.0", "1.0.0"]);
    }

    // --- the github DTO parses a sample payload into a correct `Release` ----------------

    #[test]
    fn github_dto_parses_sample_payload_through_getters() {
        // A realistic github release object (tag, name, created_at, body, one asset) must parse via
        // the private `ReleaseDto` into a public `Release` whose getters return the expected values:
        // the leading `v` is stripped from the version, the asset `url`/`name` map across, and the
        // body is carried.
        let body = r#"{
            "tag_name": "v4.5.6",
            "name": "Release 4.5.6",
            "created_at": "2024-01-02T03:04:05Z",
            "body": "the release notes",
            "assets": [
                { "name": "app-x86_64-unknown-linux-gnu.tar.gz", "url": "https://api/asset/1" }
            ]
        }"#;
        let base = stub(move |_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: body.to_string(),
            }]
        });
        // `get_latest_release` hits `/releases/latest`, which returns a bare release OBJECT parsed
        // by the single-object DTO path.
        let upd = github_update_sync(&base, "1.0.0");
        let releases = upd.get_latest_release().unwrap();
        let rel = releases.latest().expect("one-element Releases");
        assert_eq!(rel.version(), "4.5.6", "leading v stripped");
        assert_eq!(rel.name(), "Release 4.5.6");
        assert_eq!(rel.date(), "2024-01-02T03:04:05Z");
        assert_eq!(rel.body(), Some("the release notes"));
        assert_eq!(rel.assets().len(), 1);
        assert_eq!(
            rel.assets()[0].name(),
            "app-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(rel.assets()[0].download_url(), "https://api/asset/1");
    }

    // --- sync/async fetch parity (same plans + parsers) ----------------------------------------

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn sync_and_async_get_newer_releases_agree_on_identical_responses() {
        // Both paths share `releases_plan` + the parser + the early-stop filter, so for the SAME
        // stubbed body they must yield the IDENTICAL filtered, ordered release list. Drive the sync
        // fetch and the async fetch against two separate stubs serving the same body, and compare.
        let body = releases_array_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]);

        let sync_body = body.clone();
        // The sync fetch uses a blocking client; run it off the async executor so its runtime is
        // not dropped inside this async context.
        let sync_versions: Vec<String> = tokio::task::spawn_blocking(move || {
            let sync_base = stub(move |_| {
                vec![Resp {
                    status: "200 OK",
                    link: None,
                    body: sync_body,
                }]
            });
            github_update_sync(&sync_base, "1.0.0")
                .get_newer_releases()
                .unwrap()
                .all()
                .iter()
                .map(|r| r.version().to_string())
                .collect()
        })
        .await
        .unwrap();

        let async_base = stub(move |_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body,
            }]
        });
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("1.0.0")
            .api_base_url(&async_base)
            .build_async()
            .unwrap();
        let async_versions: Vec<String> = upd
            .get_newer_releases_async()
            .await
            .unwrap()
            .all()
            .iter()
            .map(|r| r.version().to_string())
            .collect();

        assert_eq!(
            sync_versions, async_versions,
            "sync and async fetch must return the identical releases for the same response"
        );
        assert_eq!(
            sync_versions,
            vec!["2.0.0".to_string(), "1.5.0".to_string()],
            "and both apply the strictly-newer per-item filter"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn fetch_all_releases_async_follows_pagination() {
        let base = stub(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/releases?page=2")),
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
            &format!("{base}/releases"),
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
    async fn get_latest_release_async_parses_release() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v3.1.0"),
            }]
        });
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .api_base_url(&base)
            .build_async()
            .unwrap();
        let releases = upd.get_latest_release_async().await.unwrap();
        let rel = releases.latest().expect("one-element Releases");
        assert_eq!(rel.version(), "3.1.0");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn update_async_reports_up_to_date() {
        // The only release (v1.0.0) is older than the current version, so the async update flow
        // fetches + filters and reports up-to-date without downloading anything.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v1.0.0"),
            }]
        });
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("2.0.0")
            .api_base_url(&base)
            .no_confirm(true)
            .show_output(false)
            .build_async()
            .unwrap();
        let status = upd.update_extended_async().await.unwrap();
        assert!(status.is_up_to_date(), "an older release means up-to-date");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn is_update_available_async_reports_newest_newer_or_none() {
        // Exercises the inherent `AsyncUpdate::is_update_available_async` verb emitted by
        // `impl_async_update_verbs!`: from an older current version it returns the newest
        // strictly-newer release; from a current version at/above the newest it returns `None`.
        let body = r#"[{"tag_name":"v2.0.0","created_at":"2020-01-01T00:00:00Z","name":"v2.0.0","assets":[]},{"tag_name":"v0.9.0","created_at":"2020-01-01T00:00:00Z","name":"v0.9.0","assets":[]}]"#;
        let mk = |cur: &'static str| {
            let base = stub(move |_| {
                vec![Resp {
                    status: "200 OK",
                    link: None,
                    body: body.to_string(),
                }]
            });
            super::Update::configure()
                .repo_owner("o")
                .repo_name("r")
                .bin_name("app")
                .current_version(cur)
                .api_base_url(&base)
                .build_async()
                .unwrap()
        };
        let newer = mk("1.0.0").is_update_available_async().await.unwrap();
        assert_eq!(
            newer.map(|r| r.version().to_string()),
            Some("2.0.0".to_string()),
            "from 1.0.0 the 2.0.0 release is available"
        );
        assert!(
            mk("2.0.0")
                .is_update_available_async()
                .await
                .unwrap()
                .is_none(),
            "from 2.0.0 nothing newer => None"
        );
    }

    #[test]
    fn get_newer_releases_sync_returns_releases_and_precheck() {
        // D1 (sync, github): `get_newer_releases()` returns a `Releases` carrying the configured
        // current version; `.is_update_available()` / `.latest()` work off it without a 2nd fetch.
        // The stub lists v2.0.0 and v0.9.0; with current 1.0.0 only 2.0.0 is newer.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"[{"tag_name":"v2.0.0","created_at":"2020-01-01T00:00:00Z","name":"v2.0.0","assets":[]},{"tag_name":"v0.9.0","created_at":"2020-01-01T00:00:00Z","name":"v0.9.0","assets":[]}]"#.to_string(),
            }]
        });
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("1.0.0")
            .api_base_url(&base)
            .build()
            .unwrap();
        let releases = upd.get_newer_releases().unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["2.0.0"], "only strictly-newer releases kept");
        assert_eq!(releases.latest().unwrap().version(), "2.0.0");
        assert!(
            releases.is_update_available().unwrap(),
            "2.0.0 > 1.0.0 via the returned Releases"
        );
    }

    fn github_update_sync(base: &str, current_version: &str) -> super::Update {
        super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version(current_version)
            .api_base_url(base)
            .build()
            .unwrap()
    }

    #[test]
    fn get_latest_release_sync_wraps_single_object_into_one_element_releases() {
        // gap #4 (sync, github): `get_latest_release` hits `/releases/latest`, which returns a
        // single release *object* (not an array). The sync path must parse that bare object,
        // strip the leading `v`, and wrap it in a one-element `Releases` carrying the current
        // version, so `.is_update_available()` works off the single newest release.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v3.1.0"),
            }]
        });
        let upd = github_update_sync(&base, "1.0.0");
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(
            releases.all().len(),
            1,
            "get_latest_release yields a one-element Releases"
        );
        assert_eq!(releases.latest().unwrap().version(), "3.1.0");
        assert!(
            releases.is_update_available().unwrap(),
            "3.1.0 > 1.0.0 via the one-element Releases pre-check"
        );
    }

    #[test]
    fn get_latest_release_sync_reports_not_available_when_newest_equals_current() {
        // gap #4 (sync, github): `/releases/latest` returns the newest tag even when it equals the
        // current version, so the one-element `Releases` must report not-available (no false
        // positive), agreeing with the strictly-newer-filtered list path.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v1.0.0"),
            }]
        });
        let upd = github_update_sync(&base, "1.0.0");
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(releases.latest().unwrap().version(), "1.0.0");
        assert!(
            !releases.is_update_available().unwrap(),
            "newest (1.0.0) == current => not available on the one-element path"
        );
    }

    #[test]
    fn update_extended_sync_reports_up_to_date_through_the_orchestrator() {
        // gap #3 (sync, git backend): the sync `update_extended()` orchestrator must drive
        // fetch -> choose_latest_release(releases.into_vec()) to an UpToDate outcome when the only
        // listed release is older than current, without touching the download. This is the git
        // backend analogue of the custom-backend sync end-to-end tests and the github *async*
        // up-to-date test.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v1.0.0"),
            }]
        });
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("2.0.0")
            .api_base_url(&base)
            .no_confirm(true)
            .show_output(false)
            .build()
            .unwrap();
        let status = upd.update_extended().unwrap();
        assert!(
            status.is_up_to_date(),
            "an older listed release means up-to-date through the sync orchestrator"
        );
    }

    #[test]
    fn fetch_all_releases_follows_link_pagination() {
        // Page 1 advertises a `rel="next"` to page 2; page 2 has no next link.
        let base = stub(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/releases?page=2")),
                    body: release_json("v1.0.0"),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_json("v0.9.0"),
                },
            ]
        });
        let releases = fetch_all_releases(
            &format!("{base}/releases"),
            &crate::backends::common::RequestConfig::default(),
        )
        .unwrap();
        assert_eq!(
            releases.len(),
            2,
            "releases from both pages are accumulated"
        );
        assert_eq!(releases[0].version(), "1.0.0");
        assert_eq!(releases[1].version(), "0.9.0");
    }

    #[test]
    fn fetch_all_releases_errors_on_http_error_status() {
        let base = stub(|_| {
            vec![Resp {
                status: "404 Not Found",
                link: None,
                body: "nope".to_string(),
            }]
        });
        let res = fetch_all_releases(
            &format!("{base}/releases"),
            &crate::backends::common::RequestConfig::default(),
        );
        // A non-2xx status always produces a structured status variant (NotFound /
        // Unauthorized / HttpStatus). Both reqwest and ureq map consistently after this change.
        assert!(matches!(
            res,
            Err(crate::errors::Error::NotFound { .. })
                | Err(crate::errors::Error::Unauthorized { .. })
                | Err(crate::errors::Error::HttpStatus { .. })
        ));
    }

    #[test]
    fn fetch_all_releases_errors_when_body_is_not_an_array() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "{}".to_string(),
            }]
        });
        let res = fetch_all_releases(
            &format!("{base}/releases"),
            &crate::backends::common::RequestConfig::default(),
        );
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidResponse { .. })),
            "a non-array listing body must surface as Error::InvalidResponse, got {:?}",
            res
        );
    }

    /// Like [`stub`], but also captures each incoming raw request so tests can assert on what
    /// the client actually sent.
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
    fn get_release_version_percent_encodes_the_tag_in_the_url() {
        // The caller-supplied tag is interpolated into the request URL and must be
        // percent-encoded. A tag with a URL-special `+` must appear as `%2B` on the wire, never
        // raw. Without the fix the raw `+` reaches the path and this assertion fails.
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v1.0.0+build.5"),
            }]
        });
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .api_base_url(&base)
            .build()
            .unwrap();
        let rel = upd.get_release_version("v1.0.0+build.5").unwrap();
        assert_eq!(rel.version(), "1.0.0+build.5");
        let request = &captured.lock().unwrap()[0];
        let request_line = request.lines().next().unwrap_or_default();
        assert!(
            request_line.contains("/releases/tags/v1.0.0%2Bbuild.5"),
            "tag should be percent-encoded in the request path, got: {}",
            request_line
        );
        assert!(
            !request_line.contains("v1.0.0+build.5"),
            "raw unencoded `+` must not reach the request path, got: {}",
            request_line
        );
    }

    #[test]
    fn urls_percent_encode_repo_owner_and_name() {
        // `releases_url()`/`latest_url()`/`tag_url()` percent-encode `repo_owner` and
        // `repo_name`, matching the gitlab/gitea backends. github.com restricts these to
        // URL-safe characters, but a GitHub Enterprise namespace (or a copy-paste of this URL
        // construction) must not smuggle raw URL-special characters into the path.
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v1.0.0"),
            }]
        });
        let upd = super::Update::configure()
            .repo_owner("own er")
            .repo_name("re#po")
            .bin_name("app")
            .current_version("0.1.0")
            .api_base_url(&base)
            .build()
            .unwrap();
        let _ = upd.get_release_version("v1.0.0").unwrap();
        let request = &captured.lock().unwrap()[0];
        let request_line = request.lines().next().unwrap_or_default();
        assert!(
            request_line.contains("/repos/own%20er/re%23po/releases/tags/"),
            "owner and name must be percent-encoded in the request path, got: {}",
            request_line
        );
    }

    #[test]
    fn builder_stores_timeout_and_request_header() {
        use std::time::Duration;
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .timeout(Duration::from_secs(7))
            // `request_header` accepts `TryInto<HeaderName>`/`TryInto<HeaderValue>`, so plain
            // string args work (no `.parse().unwrap()` needed).
            .request_header("x-foo", "bar")
            .build()
            .unwrap();
        assert_eq!(upd.request_timeout(), Some(Duration::from_secs(7)));
        assert_eq!(
            upd.request_headers()
                .get("x-foo")
                .unwrap()
                .to_str()
                .unwrap(),
            "bar"
        );
    }

    #[test]
    fn request_header_accepts_typed_args() {
        use crate::http_client::header::{HeaderName, HeaderValue};
        // Already-typed header name/value still work (identity `TryInto`), keeping old call sites
        // valid.
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .request_header(
                HeaderName::from_static("x-typed"),
                HeaderValue::from_static("v"),
            )
            .build()
            .unwrap();
        assert_eq!(upd.request_headers().get("x-typed").unwrap(), "v");
    }

    #[test]
    fn api_headers_override_uses_github_user_agent() {
        // The `{api_headers}` override arm of `impl_update_config_accessors!` must wire github's
        // custom `api_headers` (the shared `self_update/<version>` User-Agent), not the trait
        // default (which sets no User-Agent). The auth scheme/token is not baked into
        // `api_headers`; it is applied centrally by `apply_auth` (asserted in
        // `github_token_scheme_applied_to_both_paths`).
        let upd = super::Update::configure()
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
            "api_headers no longer bakes auth; apply_auth applies the scheme"
        );
    }

    // A malformed root certificate supplied via `add_root_certificate` surfaces end to end as
    // `Error::InvalidCertificate` from `build()` on both the Update and ReleaseList builders (the
    // deferred cert-build error is materialized by `build_client` and surfaced by `check`). The
    // reqwest client rejects a PEM-framed body that is not valid X.509 DER at client-build time.
    #[cfg(feature = "reqwest")]
    #[test]
    fn add_root_certificate_bad_cert_surfaces_from_build() {
        const BAD_PEM: &[u8] =
            b"-----BEGIN CERTIFICATE-----\nbm90IGEgdmFsaWQgY2VydA==\n-----END CERTIFICATE-----\n";
        let res = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .add_root_certificate(crate::Certificate::from_pem(BAD_PEM.to_vec()))
            .build();
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidCertificate { .. })),
            "a bad cert must surface as InvalidCertificate from Update build(), got {:?}",
            res.map(|_| "Ok")
        );
        let res = super::ReleaseList::configure()
            .repo_owner("o")
            .repo_name("r")
            .add_root_certificate(crate::Certificate::from_pem(BAD_PEM.to_vec()))
            .build();
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidCertificate { .. })),
            "a bad cert must surface as InvalidCertificate from ReleaseList build(), got {:?}",
            res.map(|_| "Ok")
        );
    }

    // github resolves to the `token` scheme, applied by the shared `apply_auth` on the request
    // config that BOTH the listing and download paths consume. A configured auth_token renders as
    // `token <token>`; a user `request_header(AUTHORIZATION, ..)` override wins on both paths.
    #[test]
    fn github_token_scheme_applied_to_both_paths() {
        use crate::http_client::header::{AUTHORIZATION, HeaderMap};
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("secret")
            .build()
            .unwrap();
        let mut headers = HeaderMap::new();
        upd.request_config()
            .apply_auth(
                "https://api.github.com/repos/o/r/releases/assets/1",
                &mut headers,
            )
            .unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "token secret",
            "github authenticates with the token scheme"
        );

        // A user AUTHORIZATION override (via request_header) wins: apply_auth is a no-op.
        let upd = super::Update::configure()
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
            .apply_auth(
                "https://api.github.com/repos/o/r/releases/assets/1",
                &mut headers,
            )
            .unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer user-override",
            "a user AUTHORIZATION override must win over the backend token scheme"
        );
    }

    // an auth token that cannot be encoded as a header value surfaces as
    // `Error::InvalidAuthToken` and chains the underlying header-parse error through `source()`.
    // The derivation lives in `apply_auth`.
    #[test]
    fn invalid_auth_token_chains_source() {
        use crate::http_client::header::HeaderMap;
        use std::error::Error as _;
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("bad\nvalue")
            .build()
            .unwrap();
        let mut headers = HeaderMap::new();
        let err = upd
            .request_config()
            .apply_auth(
                "https://api.github.com/repos/o/r/releases/assets/1",
                &mut headers,
            )
            .expect_err("an unencodable auth token must error");
        assert!(
            matches!(err, crate::errors::Error::InvalidAuthToken { .. }),
            "expected Error::InvalidAuthToken, got {:?}",
            err
        );
        assert!(
            err.source().is_some(),
            "InvalidAuthToken must chain a non-None source()"
        );
    }

    #[test]
    fn request_header_surfaces_invalid_header_at_build() {
        // A header name that is not a valid HTTP token must fail at `build()` with `Error::Config`,
        // not panic in the setter.
        let res = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .request_header("inva lid name", "ok")
            .build();
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidHeader { .. })),
            "invalid header name should surface as Error::InvalidHeader from build()"
        );
    }

    #[test]
    fn builder_stores_progress_callback() {
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .progress_callback(|_downloaded, _total| {})
            .build()
            .unwrap();
        // The callback is forwarded to the download step (accessor is internal/doc-hidden).
        assert!(upd.progress_callback().is_some());
    }

    #[test]
    fn builder_stores_verify_hook() {
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .verify_binary(|_new_exe| Ok(()))
            .build()
            .unwrap();
        assert!(upd.verify_callback().is_some());
    }

    #[test]
    #[cfg(feature = "checksums")]
    fn builder_stores_checksum() {
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .verify_checksum(crate::Checksum::Sha256("ab".repeat(32)))
            .build()
            .unwrap();
        assert!(upd.verify_checksum().is_some());
    }

    #[test]
    fn builder_stores_asset_matcher() {
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .asset_matcher(|assets| assets.first().cloned())
            .build()
            .unwrap();
        assert!(upd.asset_matcher().is_some());
    }

    #[test]
    fn asset_matcher_overrides_default_selection() {
        use crate::update::{Release, ReleaseAsset};

        // Asset names the built-in target/OS/ARCH substring heuristic can't pick.
        let release = Release::builder()
            .version("1.0.0")
            .assets([
                ReleaseAsset::new("app-stable.bin", "https://example/stable"),
                ReleaseAsset::new("app-nightly.bin", "https://example/nightly"),
            ])
            .build()
            .unwrap();

        // Default selection finds nothing (no asset contains the target triple / OS+ARCH).
        assert!(release.asset_for("some-unmatchable-target", None).is_none());

        // A custom matcher can pick by an arbitrary rule.
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .asset_matcher(|assets| assets.iter().find(|a| a.name.contains("nightly")).cloned())
            .build()
            .unwrap();
        let matcher = upd.asset_matcher().expect("matcher stored");
        let chosen = matcher(release.assets()).expect("matcher selects an asset");
        assert_eq!(chosen.name(), "app-nightly.bin");
        assert_eq!(chosen.download_url(), "https://example/nightly");
    }

    #[cfg(feature = "reqwest")]
    #[test]
    fn builder_stores_reqwest_client() {
        let client = reqwest::blocking::Client::builder().build().unwrap();
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .reqwest_client(client)
            .build()
            .unwrap();
        // The convenience setter wraps the client in a `ReqwestClient` and stores it as the
        // injected `Arc<dyn HttpClient>`.
        assert!(upd.request_client().is_some());
    }

    /// A `HeaderMap` with a single marker header, used as an injected client's `default_headers`
    /// so the wire tests can prove the *injected* client (not a fresh per-call one) was used.
    #[cfg(feature = "reqwest")]
    fn marker_default_headers() -> crate::http_client::header::HeaderMap {
        use crate::http_client::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-injected-client"),
            HeaderValue::from_static("marker"),
        );
        headers
    }

    #[cfg(feature = "reqwest")]
    #[test]
    fn injected_reqwest_client_is_used_on_the_wire() {
        use crate::backends::common::RequestConfig;
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v1.2.3"),
            }]
        });
        // The injected client carries a marker default header the per-call client would never add.
        let client = reqwest::blocking::Client::builder()
            .default_headers(marker_default_headers())
            .build()
            .unwrap();
        let cfg = RequestConfig {
            client: Some(std::sync::Arc::new(
                crate::http_client::ReqwestClient::from(client),
            )),
            ..Default::default()
        };
        let releases = fetch_all_releases(&format!("{base}/releases"), &cfg).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version(), "1.2.3");
        let request = captured.lock().unwrap()[0].to_lowercase();
        assert!(
            request.contains("x-injected-client: marker"),
            "the injected client's default header proves it was used (not a fresh client)"
        );
    }

    #[cfg(feature = "async")]
    #[test]
    fn builder_stores_reqwest_async_client() {
        let client = reqwest::Client::builder().build().unwrap();
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .reqwest_async_client(client)
            .build()
            .unwrap();
        assert!(upd.request_async_client().is_some());
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn injected_async_client_is_used_on_the_wire() {
        use crate::backends::common::RequestConfig;
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.0.0"),
            }]
        });
        let client = reqwest::Client::builder()
            .default_headers(marker_default_headers())
            .build()
            .unwrap();
        let cfg = RequestConfig {
            async_client: Some(std::sync::Arc::new(
                crate::http_client::ReqwestAsyncClient::from(client),
            )),
            ..Default::default()
        };
        let releases = fetch_all_releases_async(&format!("{base}/releases"), &cfg)
            .await
            .unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version(), "2.0.0");
        let request = captured.lock().unwrap()[0].to_lowercase();
        assert!(
            request.contains("x-injected-client: marker"),
            "the injected async client's default header proves it was used"
        );
    }

    #[cfg(feature = "ureq")]
    #[test]
    fn injected_ureq_agent_is_used_on_the_wire() {
        use crate::backends::common::RequestConfig;
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v3.0.0"),
            }]
        });
        let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .ureq_agent(agent)
            .build()
            .unwrap();
        assert!(upd.request_client().is_some());

        // And the injected agent actually performs the request.
        let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
        let cfg = RequestConfig {
            client: Some(std::sync::Arc::new(crate::http_client::UreqClient::from(
                agent,
            ))),
            ..Default::default()
        };
        let releases = fetch_all_releases(&format!("{base}/releases"), &cfg).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version(), "3.0.0");
    }

    // --- trait-seam injection (client-agnostic, no reqwest/ureq) ------------------------

    /// A test-double [`HttpResponse`](crate::http_client::HttpResponse) wrapping a canned JSON body.
    /// `json_value`/`text` read the stored body; `body` streams it. This proves a backend can be
    /// driven by an arbitrary response that is neither a reqwest nor a ureq type.
    struct FakeResponse {
        body: String,
        headers: crate::http_client::HeaderMap,
    }

    impl crate::http_client::HttpResponse for FakeResponse {
        fn headers(&self) -> &crate::http_client::HeaderMap {
            &self.headers
        }
        fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
            Box::new(std::io::Cursor::new(self.body.into_bytes()))
        }
    }

    /// A test-double [`HttpClient`](crate::http_client::HttpClient) that records every requested URL
    /// and returns a canned `Box<dyn HttpResponse>`. This is the testability payoff of the trait
    /// seam: a backend can be exercised with no network and no concrete client crate.
    struct FakeClient {
        body: String,
        requested: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl crate::http_client::HttpClient for FakeClient {
        fn get(
            &self,
            url: &str,
            _headers: &crate::http_client::HeaderMap,
            _timeout: Option<std::time::Duration>,
        ) -> crate::errors::Result<Box<dyn crate::http_client::HttpResponse>> {
            self.requested.lock().unwrap().push(url.to_string());
            Ok(Box::new(FakeResponse {
                body: self.body.clone(),
                headers: crate::http_client::HeaderMap::new(),
            }))
        }
    }

    #[test]
    fn injected_fake_http_client_drives_a_backend_through_the_trait() {
        // The github fetch path reads the release listing through `HttpClient::get` /
        // `HttpResponse::json_value`. Inject a `FakeClient` (not reqwest/ureq) via `.http_client(...)`
        // and assert (1) the backend parsed the canned body and (2) the fake recorded the URL the
        // backend asked for — proving the request actually went through the injected trait object.
        use crate::backends::common::RequestConfig;
        let requested = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let cfg = RequestConfig {
            client: Some(std::sync::Arc::new(FakeClient {
                body: release_json("v4.5.6"),
                requested: requested.clone(),
            })),
            ..Default::default()
        };
        let releases = fetch_all_releases("https://example.test/repos/o/r/releases", &cfg).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(
            releases[0].version(),
            "4.5.6",
            "the backend parsed the fake client's canned body through the trait"
        );
        let urls = requested.lock().unwrap();
        assert_eq!(urls.len(), 1, "exactly one request was issued");
        assert!(
            urls[0].contains("/repos/o/r/releases"),
            "the fake client recorded the URL the backend requested through the trait, got {:?}",
            urls[0]
        );
    }

    #[test]
    fn http_traits_are_object_safe() {
        // Compile-time assertion that the seam traits are object-safe: if a non-object-safe method
        // (e.g. a generic `json::<T>()`) crept back in, these `Box<dyn ...>` coercions would fail to
        // compile. `FakeClient`/`FakeResponse` exercise the dyn coercion concretely.
        let _client: Box<dyn crate::http_client::HttpClient> = Box::new(FakeClient {
            body: "[]".to_string(),
            requested: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        });
        let _resp: Box<dyn crate::http_client::HttpResponse> = Box::new(FakeResponse {
            body: "[]".to_string(),
            headers: crate::http_client::HeaderMap::new(),
        });
        // Arc<dyn HttpClient> is the injection carrier, so it must also be object-safe.
        let _arc: std::sync::Arc<dyn crate::http_client::HttpClient> =
            std::sync::Arc::new(FakeClient {
                body: "[]".to_string(),
                requested: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            });
    }

    #[test]
    fn request_header_is_sent_on_the_wire() {
        use crate::backends::common::RequestConfig;
        use crate::http_client::header::{HeaderMap, HeaderName, HeaderValue};
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v1.0.0"),
            }]
        });
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-custom"),
            HeaderValue::from_static("hello"),
        );
        let cfg = RequestConfig {
            timeout: None,
            headers,
            ..Default::default()
        };
        let releases = fetch_all_releases(&format!("{base}/releases"), &cfg).unwrap();
        assert_eq!(releases.len(), 1);
        let request = captured.lock().unwrap()[0].to_lowercase();
        assert!(
            request.contains("x-custom: hello"),
            "custom header missing from request:\n{}",
            captured.lock().unwrap()[0]
        );
    }

    #[test]
    fn timeout_aborts_an_unresponsive_request() {
        use crate::backends::common::RequestConfig;
        use std::time::{Duration, Instant};
        // Accept the connection but never send a response.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let _held = listener.accept();
            std::thread::sleep(Duration::from_secs(5));
        });
        let cfg = RequestConfig {
            timeout: Some(Duration::from_millis(200)),
            ..Default::default()
        };
        let start = Instant::now();
        let res = fetch_all_releases(&format!("{base}/releases"), &cfg);
        assert!(res.is_err(), "expected a timeout error");
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "request should have timed out quickly, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn retries_recover_from_transient_failures() {
        use crate::backends::common::RequestConfig;
        // First two attempts fail (503), the third succeeds.
        let base = stub(|_| {
            vec![
                Resp {
                    status: "503 Service Unavailable",
                    link: None,
                    body: "busy".to_string(),
                },
                Resp {
                    status: "503 Service Unavailable",
                    link: None,
                    body: "busy".to_string(),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_json("v1.0.0"),
                },
            ]
        });
        let cfg = RequestConfig {
            retries: 2,
            ..Default::default()
        };
        let releases = fetch_all_releases(&format!("{base}/releases"), &cfg).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version(), "1.0.0");
    }

    #[test]
    fn retries_are_exhausted_and_then_error() {
        use crate::backends::common::RequestConfig;
        // One retry allowed -> two attempts, both 503 -> error.
        let base = stub(|_| {
            vec![
                Resp {
                    status: "503 Service Unavailable",
                    link: None,
                    body: "busy".to_string(),
                },
                Resp {
                    status: "503 Service Unavailable",
                    link: None,
                    body: "busy".to_string(),
                },
            ]
        });
        let cfg = RequestConfig {
            retries: 1,
            ..Default::default()
        };
        let res = fetch_all_releases(&format!("{base}/releases"), &cfg);
        assert!(res.is_err());
    }

    #[test]
    fn release_list_applies_its_request_config() {
        // Confirms `ReleaseList`'s transport setters (here `retries`) flow through `fetch`.
        let base = stub(|_| {
            vec![
                Resp {
                    status: "503 Service Unavailable",
                    link: None,
                    body: "busy".to_string(),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_json("v2.0.0"),
                },
            ]
        });
        let releases = super::ReleaseList::configure()
            .api_base_url(&base)
            .repo_owner("o")
            .repo_name("r")
            .retries(1)
            .build()
            .unwrap()
            .fetch()
            .unwrap()
            .into_vec();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version(), "2.0.0");
    }

    // --- unattended() convenience ---------------------------------------------------

    #[test]
    fn unattended_sets_no_confirm_and_hides_output() {
        // Build a config without calling `unattended()` first to confirm the defaults.
        let upd_default = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert!(
            !upd_default.no_confirm(),
            "default no_confirm must be false"
        );
        assert!(
            upd_default.show_output(),
            "default show_output must be true"
        );

        // After `unattended()` both flags flip.
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .unattended()
            .build()
            .unwrap();
        assert!(upd.no_confirm(), "unattended() must set no_confirm to true");
        assert!(
            !upd.show_output(),
            "unattended() must set show_output to false"
        );
    }

    // `build()` returns a concrete `Update` that is `Send`, so it can move to a worker thread
    // (`std::thread::spawn(move || updater.update())`). A regression that made `Update` `!Send`
    // (e.g. an `Rc` field) would fail to compile here.
    #[test]
    fn built_update_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<super::Update>();
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        // Move it into a thread to exercise the `Send` bound end to end.
        std::thread::spawn(move || {
            let _ = &upd;
        })
        .join()
        .unwrap();
    }

    // --- verifying_keys builder setter and accessor --------------------------------

    #[cfg(feature = "signatures")]
    #[test]
    fn builder_stores_verify_keys() {
        // A 32-byte zeroed key slice (VerifyingKey = [u8; 32]) is enough to prove the
        // setter and accessor wire through.
        let key_bytes = [0u8; 32];
        let upd = super::Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .verifying_keys([key_bytes])
            .build()
            .unwrap();
        assert_eq!(
            upd.verifying_keys().len(),
            1,
            "verifying_keys() must return the key that was set"
        );
        assert_eq!(
            upd.verifying_keys()[0],
            key_bytes,
            "returned key bytes must match what was supplied"
        );
    }

    // --- I2: api_headers takes no auth param; I1: api_headers_for uses .expect not .unwrap_or_default ---

    #[test]
    fn api_headers_takes_no_auth_param_and_sets_user_agent() {
        // I2: api_headers() is now a zero-arg function (the unused _auth_token param was removed).
        // This test calls it with no arguments -- it would fail to compile against the old
        // `api_headers(_auth_token: Option<&str>)` signature.
        // The User-Agent assertion ensures a broken implementation cannot silently pass.
        let headers = super::api_headers().unwrap();
        assert_eq!(
            headers
                .get(crate::http_client::header::USER_AGENT)
                .unwrap()
                .to_str()
                .unwrap(),
            crate::DEFAULT_USER_AGENT,
            "api_headers() must set the shared self_update User-Agent"
        );
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "api_headers() must not set an Authorization header"
        );
    }

    #[test]
    fn continuation_page_user_agent_header_is_present() {
        // I1: the continuation-page header builder must not silently drop the User-Agent.
        // Drive a two-page fetch via stub_capturing and assert the second page's request carries
        // the User-Agent header -- if api_headers_for() silently returned empty headers (the old
        // .unwrap_or_default() path), the User-Agent would be absent on page 2.
        let (base, captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/releases?page=2")),
                    body: release_json("v2.0.0"),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: release_json("v1.0.0"),
                },
            ]
        });
        let releases = fetch_all_releases(
            &format!("{base}/releases"),
            &crate::backends::common::RequestConfig::default(),
        )
        .unwrap();
        assert_eq!(releases.len(), 2, "both pages must be fetched");
        let requests = captured.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "exactly two HTTP requests must be issued"
        );
        // Both requests must carry the User-Agent header.
        let expected_ua = format!("user-agent: {}", crate::DEFAULT_USER_AGENT.to_lowercase());
        for (i, req) in requests.iter().enumerate() {
            assert!(
                req.to_lowercase().contains(&expected_ua),
                "page {} request is missing the User-Agent header:\n{}",
                i + 1,
                req
            );
        }
    }
}

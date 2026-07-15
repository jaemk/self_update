/*!
gitea releases
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

/// Gitea release-asset JSON shape (download URL is `browser_download_url`). Private DTO converted
/// into the public [`ReleaseAsset`]; keeping it private keeps `Deserialize` out of `ReleaseAsset`'s
/// public API.
#[derive(Deserialize)]
struct AssetDto {
    name: Option<String>,
    browser_download_url: Option<String>,
}

impl AssetDto {
    fn into_asset(self) -> Result<ReleaseAsset> {
        let download_url = self
            .browser_download_url
            .ok_or_else(|| Error::missing_asset_field("browser_download_url"))?;
        let name = self
            .name
            .ok_or_else(|| Error::missing_asset_field("name"))?;
        Ok(ReleaseAsset::new(name, download_url))
    }
}

/// Gitea release JSON shape. Private DTO deserialized directly from the response bytes, then
/// converted into the public [`Release`].
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
    host: Option<String>,
    repo_owner: Option<String>,
    repo_name: Option<String>,
    target: Option<String>,
    auth_token: Option<String>,
    request: RequestConfig,
}
impl ReleaseListBuilder {
    /// Required. Set the base URL of the Gitea instance, e.g. `https://gitea.example.com`.
    ///
    /// Unlike `gitlab` (which defaults to `https://gitlab.com`), Gitea has no canonical public
    /// host, so `build()` errors if this is not set.
    ///
    /// Pass the instance host only (scheme + host, no trailing slash); the crate appends the
    /// `/api/v1/...` path itself. Do not include `/api/v1`.
    ///
    /// Note: this setter differs from `github`'s `api_base_url` (which takes the full API base
    /// URL including any path prefix, e.g. `https://api.github.com`) and from `s3`'s `endpoint`
    /// (which selects the S3 service type and endpoint). Each backend's custom-URL setter has a
    /// different shape matching its API's structure.
    pub fn host(&mut self, url: impl Into<String>) -> &mut Self {
        self.host = Some(url.into());
        self
    }

    /// Required. Set the repo owner, used to build a gitea api url
    pub fn repo_owner(&mut self, owner: impl Into<String>) -> &mut Self {
        self.repo_owner = Some(owner.into());
        self
    }

    /// Required. Set the repo name, used to build a gitea api url
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

    /// Set the authorization token, used in requests to the gitea api url
    ///
    /// This is to support private repos where you need a gitea auth token.
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
        // Thread the auth token + gitea's `token` scheme (the default) into the request so the
        // shared `apply_auth` applies it on the listing path (honoring a user override).
        let mut request = self.request.clone();
        request.auth_scheme = crate::backends::common::AuthScheme::Token;
        request.auth_token = self.auth_token.clone();
        request.auth_base_host = self
            .host
            .as_deref()
            .and_then(crate::backends::common::host_of);
        request.build_client();
        request.check()?;
        Ok(ReleaseList {
            host: if let Some(ref host) = self.host {
                host.to_owned()
            } else {
                return Err(Error::MissingField { field: "host" });
            },
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

/// `ReleaseList` provides a builder api for querying a gitea repo,
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
            "{}/api/v1/repos/{}/{}/releases",
            self.host,
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
            "{}/api/v1/repos/{}/{}/releases",
            self.host,
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

/// `gitea::Update` builder
///
/// Configure download and installation from
/// `https://<gitea-host>/api/v1/repos/<repo_owner>/<repo_name>/releases`
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

    /// Required. Set the base URL of the Gitea instance, e.g. `https://gitea.example.com`.
    ///
    /// Unlike `gitlab` (which defaults to `https://gitlab.com`), Gitea has no canonical public
    /// host, so `build()` errors if this is not set.
    ///
    /// Pass the instance host only (scheme + host, no trailing slash); the crate appends the
    /// `/api/v1/...` path itself. Do not include `/api/v1`.
    ///
    /// Note: this setter differs from `github`'s `api_base_url` (which takes the full API base
    /// URL including any path prefix, e.g. `https://api.github.com`) and from `s3`'s `endpoint`
    /// (which selects the S3 service type and endpoint). Each backend's custom-URL setter has a
    /// different shape matching its API's structure.
    pub fn host(&mut self, url: impl Into<String>) -> &mut Self {
        self.host = Some(url.into());
        self
    }

    /// Required. Set the repo owner, used to build a gitea api url
    pub fn repo_owner(&mut self, owner: impl Into<String>) -> &mut Self {
        self.repo_owner = Some(owner.into());
        self
    }

    /// Required. Set the repo name, used to build a gitea api url
    pub fn repo_name(&mut self, name: impl Into<String>) -> &mut Self {
        self.repo_name = Some(name.into());
        self
    }

    impl_common_builder_setters!();

    /// Internal: validate config into a concrete `Update`. Shared by `build` / `build_async`.
    fn build_update(&self) -> Result<Update> {
        Ok(Update {
            host: if let Some(ref host) = self.host {
                host.to_owned()
            } else {
                return Err(Error::MissingField { field: "host" });
            },
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
                let mut resolved = self.common.build()?;
                // Gitea authenticates with `token <token>`; set the scheme explicitly rather than
                // relying on `AuthScheme::default()` (gitlab overrides to `Bearer` the same way).
                resolved.request.auth_scheme = crate::backends::common::AuthScheme::Token;
                resolved.request.auth_base_host = self
                    .host
                    .as_deref()
                    .and_then(crate::backends::common::host_of);
                resolved
            },
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

/// Updates to a specified or latest release distributed via gitea
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
            "{}/api/v1/repos/{}/{}/releases",
            self.host,
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        )
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
        let releases = run_paginated(newest_plan(&self.releases_url())?, &self.common.request)?;
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
    fn api_headers(&self, _auth_token: Option<&str>) -> Result<header::HeaderMap> {
        api_headers()
    }
});

/// Transport-free plan to fetch the paginated `releases` array (Gitea format), parsing each page
/// via the private `ReleaseDto` and following `Link: rel="next"`. See github's
/// `releases_plan` for the `stop_at` per-item filter contract.
///
/// `stop_at` filters per-item: when `Some(current_version)` each release that is not strictly
/// newer than it is omitted from the collected list, but pagination continues to subsequent pages
/// regardless (a backport release -- older semver, newer creation date -- must not halt the walk
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

/// Transport-free plan for the newest release: Gitea has no `/releases/latest`, so the listing's
/// first element (newest-first order) is "latest". Fetches just the first page (no pagination).
/// Entries with non-semver rolling tags (`nightly`, ...) are skipped like the full listing does,
/// so "newest" is the first entry the updater can actually compare.
fn newest_plan(base_url: &str) -> Result<PageRequest<Release>> {
    let headers = api_headers()?;
    Ok(PageRequest {
        url: first_page_url(base_url),
        headers,
        parse: Box::new(|body, _resp_headers| {
            let dtos: Vec<ReleaseDto> =
                serde_json::from_slice(body).map_err(|e| Error::InvalidResponse {
                    source: Box::new(e),
                })?;
            for dto in dtos {
                match dto.into_release() {
                    Ok(release) => return Ok(Page::last(vec![release])),
                    Err(e @ Error::SemVer(_)) => {
                        log::debug!("self_update: skipping listed release: {e}");
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(Error::NoReleaseFound { target: None })
        }),
    })
}

/// Transport-free plan to fetch a single release *object* (the `.../releases/tags/{ver}` endpoint).
fn single_plan(url: String) -> Result<PageRequest<Release>> {
    let headers = api_headers()?;
    Ok(PageRequest {
        url,
        headers,
        parse: Box::new(|body, _resp_headers| {
            // An unparseable body is `InvalidResponse`, matching the paginated listing parser.
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
            run_paginated_async(newest_plan(&self.releases_url())?, &self.common.request).await?;
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

/// Build gitea's base request headers (its User-Agent). The Authorization header is applied
/// centrally by the shared [`apply_auth`](crate::backends::common::RequestConfig::apply_auth) using
/// gitea's `token` scheme on both the listing and download paths, honoring a user override.
fn api_headers() -> Result<header::HeaderMap> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        crate::DEFAULT_USER_AGENT
            .parse()
            .expect("gitea invalid user-agent"),
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
        crate::backends::run_paginated_async(super::releases_plan(base_url, None)?, req).await
    }

    // The single-release endpoint (`.../releases/tags/{ver}`) surfaces an unparseable body as
    // `InvalidResponse`, matching the paginated listing parser (previously `Error::Json`).
    #[test]
    fn single_plan_parse_failure_is_invalid_response() {
        let req =
            super::single_plan("https://example.test/releases/tags/1.0.0".to_string()).unwrap();
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

    // Gitea's "latest" is the listing's first entry; a rolling tag in that slot must be
    // skipped so "newest" is the first release the updater can actually compare.
    #[test]
    fn newest_plan_skips_non_semver_tags() {
        let req = super::newest_plan("https://example.test/releases").unwrap();
        let body = releases_json(&["nightly", "v1.2.3", "v1.0.0"]);
        let page = (req.parse)(body.as_bytes(), &crate::http_client::HeaderMap::new()).unwrap();
        let versions: Vec<&str> = page.items.iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["1.2.3"]);
    }

    // With only non-semver tags there is nothing the updater can compare: NoReleaseFound.
    #[test]
    fn newest_plan_with_only_non_semver_tags_is_no_release_found() {
        let req = super::newest_plan("https://example.test/releases").unwrap();
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

    /// A JSON array of one release (used by the async pagination and latest-release tests).
    fn release_json(tag: &str) -> String {
        format!(
            r#"[{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[],"body":null}}]"#
        )
    }

    /// A JSON array of several releases (one object per `tag`), used by the
    /// `get_newer_releases_async` filtering test.
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

    /// A bare JSON release object (not wrapped in an array). Gitea's `get_release_version[_async]`
    /// hits `/tags/{ver}`, which returns a single release object, so this is parsed directly.
    fn release_obj_json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[],"body":null}}"#
        )
    }

    #[cfg(feature = "async")]
    fn gitea_update(base: &str, current_version: &str) -> super::AsyncUpdate {
        Update::configure()
            .host(base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version(current_version)
            .build_async()
            .unwrap()
    }

    /// Build a `ReleaseUpdate` (sync) gitea `Update` pointed at the loopback stub.
    fn gitea_update_sync(base: &str, current_version: &str) -> Update {
        Update::configure()
            .host(base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version(current_version)
            .build()
            .unwrap()
    }

    // --- Sync `Releases`-returning fetch coverage (gap #2) ------------------------------------
    //
    // The async fetch methods were exercised above; these pin the *sync* `ReleaseUpdate` fetch
    // methods on the same loopback stub, proving they wrap into a `Releases` carrying the
    // configured current version and that `latest()`/`all()`/`is_update_available()` work off it.

    #[test]
    fn get_latest_release_sync_wraps_newest_into_one_element_releases() {
        // `get_latest_release` parses the first element of the gitea releases array and wraps it
        // in a one-element `Releases` carrying the configured current version, so the pre-check
        // works directly off that single release without a second fetch.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.5.0"),
            }]
        });
        let upd = gitea_update_sync(&base, "1.0.0");
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
    fn get_newer_releases_sync_returns_releases_and_filters_to_newer() {
        // `get_newer_releases` (sync) follows pagination, filters to strictly-newer releases,
        // wraps them in a `Releases`, and the returned `Releases` agrees on availability with the
        // list path.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
            }]
        });
        let upd = gitea_update_sync(&base, "1.0.0");
        let releases = upd.get_newer_releases().unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["2.0.0", "1.5.0"],
            "only releases strictly newer than the current version are kept, in order"
        );
        assert_eq!(releases.latest().unwrap().version(), "2.0.0");
        assert!(
            releases.is_update_available().unwrap(),
            "the list path reports an update available when something newer exists"
        );
    }

    #[test]
    fn get_newer_releases_sync_reports_no_update_when_up_to_date() {
        // gap #4 (sync, gitea): when nothing is strictly newer, the strictly-newer-filtered list
        // path is empty and `is_update_available()` must report false (no panic, no error).
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v1.0.0", "v0.9.0"]),
            }]
        });
        let upd = gitea_update_sync(&base, "1.0.0");
        let releases = upd.get_newer_releases().unwrap();
        assert!(releases.all().is_empty(), "no newer release => empty list");
        assert!(
            !releases.is_update_available().unwrap(),
            "empty list => no update available"
        );
    }

    #[test]
    fn get_latest_release_sync_agrees_with_list_path_when_newest_equals_current() {
        // gap #4 (sync, gitea): the one-element `get_latest_release` path wraps the newest tag even
        // when it equals current, so its `is_update_available()` must report false; the
        // strictly-newer-filtered `get_newer_releases` path must agree (empty => false). Both
        // paths must answer "not available" off the same stubbed listing.
        let make_body = || {
            // get_latest_release reads the FIRST element; place the newest (equal to current) first.
            releases_json(&["v1.0.0", "v0.9.0"])
        };

        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: make_body(),
            }]
        });
        let upd = gitea_update_sync(&base, "1.0.0");
        let single = upd.get_latest_release().unwrap();
        assert_eq!(single.latest().unwrap().version(), "1.0.0");
        assert!(
            !single.is_update_available().unwrap(),
            "get_latest_release: newest (1.0.0) == current => not available"
        );

        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: make_body(),
            }]
        });
        let upd = gitea_update_sync(&base, "1.0.0");
        let list = upd.get_newer_releases().unwrap();
        // F1 distinction: the RAW `get_latest_release` path keeps the newest tag (latest() is
        // Some, above), but the strictly-newer-FILTERED `get_newer_releases` path drops it
        // entirely — so here the list is empty and `latest()` is None, not merely "not newer".
        // Asserting emptiness (not just `!is_update_available()`) pins the filter: a regression
        // that stopped filtering would still report `!is_update_available()` but would leave
        // latest() == Some("1.0.0"), which this catches.
        assert!(
            list.all().is_empty(),
            "get_newer_releases: nothing strictly newer => filtered list is empty"
        );
        assert!(
            list.latest().is_none(),
            "get_newer_releases: empty filtered list => latest() is None"
        );
        assert!(
            !list.is_update_available().unwrap(),
            "get_newer_releases: nothing strictly newer => not available (agrees with single path)"
        );
    }

    /// Like [`stub`], but also captures each incoming raw request so tests can assert on what the
    /// client actually sent (e.g. whether page 2 was ever requested).
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

    // --- gitea git release-scan early-stop (per-backend parser wiring) -----------------
    //
    // The early-stop lives in shared code, but the gitea parser (`from_release` + the shared
    // `release_array_page`) wires `stop_at` itself. These pin that wiring: the parser must set
    // `Page::stop` on the first release NOT strictly newer than current and the driver must NOT
    // request page 2 (advertised via a `rel="next"` Link header), and the early-stopped selection
    // must match a full-walk selection.

    #[test]
    fn get_newer_releases_continues_past_non_newer_releases_and_fetches_page_two() {
        // Page 1 has both newer (v2.0.0, v1.5.0) and non-newer (v1.0.0, v0.9.0) releases, with
        // a link to page 2. Non-newer releases must NOT halt pagination -- page 2 must be fetched
        // and its newer release (v3.0.0) returned alongside the newer items from page 1.
        // (The old early-stop bug would have returned only ["2.0.0", "1.5.0"] in 1 request.)
        let (base, captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v1/repos/o/r/releases?page=2")),
                    body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v3.0.0"]),
                },
            ]
        });
        let upd = gitea_update_sync(&base, "1.0.0");
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
        // Regression test for the per-item-filter pagination bug:
        // Page 1 has ONLY non-newer releases (v1.0.0, v0.9.0); page 2 has a newer release
        // (v1.5.0). With the old bug, pagination halted at v1.0.0 on page 1 and page 2 was never
        // fetched, so early = [] and early_choice = None -- not matching full_choice = Some(v1.5.0).
        // With the fix, page 2 is followed, v1.5.0 is included, and both choices agree.
        let (base, _captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v1/repos/o/r/releases?page=2")),
                    body: releases_json(&["v1.0.0", "v0.9.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v1.5.0"]),
                },
            ]
        });
        let upd = gitea_update_sync(&base, "1.0.0");
        let early = upd.get_newer_releases().unwrap().into_vec();
        let early_choice =
            crate::update::testing::choose_latest_release_for_test(early, "1.0.0").unwrap();

        // Full unfiltered walk includes all releases from both pages; the selection algorithm
        // must pick the same release as the per-item-filtered walk.
        let full: Vec<_> = ["1.0.0", "0.9.0", "1.5.0"]
            .iter()
            .map(|v| {
                crate::update::Release::builder()
                    .version(*v)
                    .build()
                    .unwrap()
            })
            .collect();
        let full_choice =
            crate::update::testing::choose_latest_release_for_test(full, "1.0.0").unwrap();
        assert_eq!(
            early_choice.map(|r| r.version().to_string()),
            full_choice.map(|r| r.version().to_string()),
            "per-item filter must select the same release as a full unfiltered walk"
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
                    link: Some(format!("{base}/api/v1/repos/o/r/releases?page=2")),
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

    // --- a realistic populated payload parses through the DTO into a `Release`
    // whose getters surface every field. The other gitea parse tests use empty `assets` and a
    // null `body`; this pins the populated mapping with gitea's distinct shapes (bare `assets`
    // array, download URL in `browser_download_url`, body in `body`).
    #[test]
    fn dto_parse_maps_populated_payload_through_getters() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"[{"tag_name":"v3.4.5","created_at":"2021-07-08T09:10:11Z","name":"My App 3.4.5","body":"the notes","assets":[{"name":"app-x86_64-linux.tar.gz","browser_download_url":"https://gitea.example/app-x86_64-linux.tar.gz"},{"name":"app-aarch64-linux.tar.gz","browser_download_url":"https://gitea.example/app-aarch64-linux.tar.gz"}]}]"#
                    .to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        let releases = upd.get_latest_release().unwrap();
        let rel = releases.latest().unwrap();
        assert_eq!(rel.version(), "3.4.5", "leading `v` stripped from tag_name");
        assert_eq!(rel.name(), "My App 3.4.5", "name surfaces from `name`");
        assert_eq!(rel.date(), "2021-07-08T09:10:11Z", "date from `created_at`");
        assert_eq!(
            rel.body(),
            Some("the notes"),
            "body surfaces from gitea's `body` field"
        );
        assert_eq!(rel.assets().len(), 2, "both `assets` entries parsed");
        assert_eq!(rel.assets()[0].name(), "app-x86_64-linux.tar.gz");
        assert_eq!(
            rel.assets()[0].download_url(),
            "https://gitea.example/app-x86_64-linux.tar.gz",
            "asset download_url comes from `browser_download_url`"
        );
        assert_eq!(rel.assets()[1].name(), "app-aarch64-linux.tar.gz");
    }

    // --- the listing `Releases` from `ReleaseList::fetch` carries NO current
    // version, so `current_version()` is `None` and `is_update_available()` errors with EXACTLY
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
        assert_eq!(
            versions,
            vec!["2.0.0", "1.0.0"],
            "into_vec() recovers the parsed releases, newest-first"
        );
    }

    // --- async sibling of `release_list_fetch_returns_listing_releases_without_current_version`:
    // `ReleaseList::fetch_async` yields the same bare listing (no current version, NoCurrentVersion
    // from `is_update_available()`, `into_vec()` recovers the releases).
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
        assert_eq!(
            versions,
            vec!["2.0.0", "1.0.0"],
            "into_vec() recovers the parsed releases, newest-first"
        );
    }

    // --- exact-variant routing on the sync transport (gitea). The sibling
    // exact-variant tests are `#[cfg(feature = "async")]` only, so the ureq lane never pins the
    // precise variant. A release object missing `tag_name` must surface as EXACTLY
    // `MissingAssetField { field: "tag_name" }`.
    #[test]
    fn sync_missing_tag_name_routes_to_missing_asset_field_exactly() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"{"created_at":"2020-01-01T00:00:00Z","name":"x","assets":[]}"#.to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        match upd.get_release_version("v1.0.0") {
            Err(crate::errors::Error::MissingAssetField { field }) => {
                assert_eq!(
                    field, "tag_name",
                    "must name the absent payload field exactly"
                );
            }
            other => panic!(
                "missing tag_name must be Error::MissingAssetField {{ field: \"tag_name\" }}, got {:?}",
                other
            ),
        }
    }

    // --- the other side of the sync-lane split (gitea): an empty top-level releases
    // array surfaces as EXACTLY `NoReleaseFound { target: None }`.
    #[test]
    fn sync_empty_array_routes_to_no_release_found_exactly() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "[]".to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        match upd.get_latest_release() {
            Err(crate::errors::Error::NoReleaseFound { target }) => {
                assert_eq!(target, None, "empty listing carries no asset target");
            }
            other => panic!(
                "empty releases array must be Error::NoReleaseFound {{ target: None }}, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn url_and_filter_target_setters_exist_on_release_list_builder() {
        // The renamed `url` / `filter_target` setters must exist on the gitea
        // `ReleaseListBuilder` and the builder must still build (gitea requires `url`).
        let _list = super::ReleaseList::configure()
            .host("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .filter_target("x86_64-unknown-linux-gnu")
            .build()
            .unwrap();
    }

    #[test]
    fn release_list_build_requires_url() {
        // gitea has no default host, so the `ReleaseList` builder must error without `url`.
        let res = super::ReleaseList::configure()
            .repo_owner("o")
            .repo_name("r")
            .build();
        assert!(matches!(
            res,
            Err(crate::errors::Error::MissingField { field: "host" })
        ));
    }

    #[test]
    fn api_headers_override_uses_gitea_user_agent() {
        // The `{api_headers}` override arm must wire gitea's custom `api_headers` (User-Agent), not
        // the trait default (which sets no User-Agent). The auth scheme/token is applied centrally
        // by `apply_auth`, not baked here.
        let upd = Update::configure()
            .host("https://gitea.example.com")
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
            "api_headers no longer bakes auth; apply_auth applies the token scheme"
        );
    }

    // gitea resolves to the `token` scheme, applied by the shared `apply_auth` on the request
    // config consumed by BOTH the listing and download paths. A user override wins.
    #[test]
    fn gitea_token_scheme_applied_to_both_paths() {
        use crate::http_client::header::{AUTHORIZATION, HeaderMap};
        #[allow(unused_imports)]
        use crate::update::UpdateInternals;
        let upd = Update::configure()
            .host("https://gitea.example.com")
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
                "https://gitea.example.com/api/v1/repos/o/r/releases",
                &mut headers,
            )
            .unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "token secret",
            "gitea authenticates with the token scheme"
        );

        // A user AUTHORIZATION override wins.
        let upd = Update::configure()
            .host("https://gitea.example.com")
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
                "https://gitea.example.com/api/v1/repos/o/r/releases",
                &mut headers,
            )
            .unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer user-override",
            "a user AUTHORIZATION override must win over the token scheme"
        );
    }

    #[test]
    fn release_list_build_surfaces_invalid_header() {
        // A bad header on the gitea `ReleaseListBuilder` must fail at `build()` via
        // `request.check()` with `Error::Config`, not panic. (The header check runs before the
        // host check, so a valid host is supplied to isolate the header failure.)
        let res = super::ReleaseList::configure()
            .host("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .request_header("inva lid", "ok")
            .build();
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidHeader { .. })),
            "invalid header must surface as Error::InvalidHeader from gitea ReleaseList build()"
        );
    }

    #[test]
    fn update_build_surfaces_invalid_header() {
        // Same deferred-header check via `CommonBuilderConfig::build` on the gitea UpdateBuilder.
        let res = Update::configure()
            .host("https://gitea.example.com")
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
    fn build_requires_url() {
        // Gitea has no default host, so `build()` must fail when `url` is not set.
        let res = Update::configure()
            .repo_owner("owner")
            .repo_name("repo")
            .bin_name("app")
            .current_version("0.1.0")
            .build();
        assert!(res.is_err(), "build must fail without a host url");
    }

    #[test]
    fn build_requires_repo_owner_and_name() {
        let missing_owner = Update::configure()
            .host("https://gitea.example.com")
            .repo_name("repo")
            .current_version("0.1.0")
            .build();
        assert!(missing_owner.is_err(), "build must fail without repo_owner");

        let missing_name = Update::configure()
            .host("https://gitea.example.com")
            .repo_owner("owner")
            .current_version("0.1.0")
            .build();
        assert!(missing_name.is_err(), "build must fail without repo_name");
    }

    #[test]
    fn releases_url_is_correct() {
        // `build_update` yields the concrete `Update` so we can check the shared base URL that both
        // the sync and async fetch paths build on.
        let upd = Update::configure()
            .host("https://gitea.example.com")
            .repo_owner("owner")
            .repo_name("repo")
            .bin_name("app")
            .current_version("0.1.0")
            .build_update()
            .unwrap();
        assert_eq!(
            upd.releases_url(),
            "https://gitea.example.com/api/v1/repos/owner/repo/releases"
        );
    }

    // --- I3: releases_url percent-encodes owner and name ------------------------------------
    //
    // `repo_owner` / `repo_name` containing URL-special characters (e.g. a space) must be
    // percent-encoded in the releases URL, matching gitlab's behavior. A space becomes %20;
    // plain alphanumeric names are unaffected.

    #[test]
    fn releases_url_encodes_owner_and_name() {
        // A space in repo_owner and repo_name must be encoded as %20 in the URL.
        // Without the fix the raw space appears in the path and breaks HTTP requests.
        let upd = Update::configure()
            .host("https://gitea.example.com")
            .repo_owner("my owner")
            .repo_name("my repo")
            .bin_name("app")
            .current_version("0.1.0")
            .build_update()
            .unwrap();
        assert_eq!(
            upd.releases_url(),
            "https://gitea.example.com/api/v1/repos/my%20owner/my%20repo/releases",
            "repo_owner and repo_name must be percent-encoded in the releases URL"
        );
    }

    #[test]
    fn release_list_fetch_encodes_owner_and_name_in_request_path() {
        // `ReleaseList::fetch` must also percent-encode owner/name in the actual HTTP request.
        // Captures the raw request line and asserts the encoded form appears in the path.
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
        assert_eq!(reqs.len(), 1, "exactly one request must be made");
        let request_line = reqs[0].lines().next().unwrap_or("");
        assert!(
            request_line.contains("/api/v1/repos/my%20owner/my%20repo/releases"),
            "owner and name must be percent-encoded in the ReleaseList::fetch request path; \
             got request line: {}",
            request_line
        );
    }

    #[test]
    fn identifier_is_wired() {
        let upd = Update::configure()
            .host("https://gitea.example.com")
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
    fn bin_name_sets_bin_path_in_archive_only_when_unset() {
        // `bin_name` auto-populates `bin_path_in_archive` (with the platform exe suffix).
        let expected = format!("app{}", std::env::consts::EXE_SUFFIX);
        let upd = Update::configure()
            .host("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(upd.bin_path_in_archive(), expected);

        // An explicit `bin_path_in_archive` set before `bin_name` is NOT overwritten.
        let upd = Update::configure()
            .host("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .bin_path_in_archive("custom/path")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(upd.bin_path_in_archive(), "custom/path");
    }

    // --- Item 4: bin_name re-derive correctness ---------------------------------------------

    #[test]
    fn bin_name_rederives_archive_path_on_second_call() {
        // Calling `.bin_name("a")` then `.bin_name("b")` must yield the archive path derived
        // from `b`, not `a`: the second call re-derives because the first was an auto-derive.
        let expected_b = format!("b{}", std::env::consts::EXE_SUFFIX);
        let upd = Update::configure()
            .host("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .bin_name("a")
            .bin_name("b")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(
            upd.bin_path_in_archive(),
            expected_b,
            "second bin_name call must re-derive the archive path from the new name"
        );
        assert_eq!(
            upd.bin_name(),
            expected_b,
            "bin_name() must reflect the second call"
        );
    }

    #[test]
    fn explicit_bin_path_survives_subsequent_bin_name_call() {
        // Calling `.bin_path_in_archive("x")` then `.bin_name("b")` must keep `"x"` — the
        // explicit set is sticky and a later `bin_name` re-derive must not overwrite it.
        let upd = Update::configure()
            .host("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .bin_path_in_archive("x")
            .bin_name("b")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(
            upd.bin_path_in_archive(),
            "x",
            "an explicit bin_path_in_archive must not be overwritten by a later bin_name call"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_parses_release() {
        // Drive `get_latest_release_async` against a loopback mock server that returns a
        // gitea-format releases JSON array, and assert the parsed version.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.5.0"),
            }]
        });
        let upd = Update::configure()
            .host(&base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build_async()
            .unwrap();
        let releases = upd.get_latest_release_async().await.unwrap();
        assert_eq!(releases.latest().unwrap().version(), "2.5.0");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn fetch_all_releases_async_follows_link_pagination() {
        // Page 1 advertises a `rel="next"` link to page 2; page 2 has no next link.
        // Both pages are accumulated and returned in order.
        let base = stub(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v1/repos/o/r/releases?page=2")),
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
            &format!("{base}/api/v1/repos/o/r/releases"),
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
        // `/tags/{ver}` returns a single release *object* (not an array). The async path must
        // parse the bare object via `from_release_gitea` and strip the leading `v` from the tag.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v4.2.1"),
            }]
        });
        let upd = gitea_update(&base, "0.1.0");
        let rel = upd.get_release_version_async("v4.2.1").await.unwrap();
        assert_eq!(rel.version(), "4.2.1");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_release_version_async_errors_on_missing_tag_name() {
        // A malformed object (no `tag_name`) must surface as a `Release` error, not panic.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"{"created_at":"2020-01-01T00:00:00Z","assets":[]}"#.to_string(),
            }]
        });
        let upd = gitea_update(&base, "0.1.0");
        let res = upd.get_release_version_async("v1.0.0").await;
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::NoReleaseFound { .. }
                    | crate::errors::Error::MissingAssetField { .. })
            ),
            "missing tag_name must surface as Error::Release, got {:?}",
            res
        );
    }

    // variant-routing (exact): a release object missing `tag_name` must surface as EXACTLY
    // `MissingAssetField { field: "tag_name" }` -- not `NoReleaseFound`. The sibling test above
    // accepts either via an `A | B` match; this pins the precise variant *and* the field name so a
    // regression that routes a payload-shape failure to the empty-listing variant (or names the
    // wrong field) is caught.
    #[cfg(feature = "async")]
    #[tokio::test]
    async fn missing_tag_name_routes_to_missing_asset_field_exactly() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"{"created_at":"2020-01-01T00:00:00Z","assets":[]}"#.to_string(),
            }]
        });
        let upd = gitea_update(&base, "0.1.0");
        let res = upd.get_release_version_async("v1.0.0").await;
        match res {
            Err(crate::errors::Error::MissingAssetField { field }) => {
                assert_eq!(
                    field, "tag_name",
                    "must name the absent payload field exactly"
                );
            }
            other => panic!(
                "missing tag_name must be Error::MissingAssetField {{ field: \"tag_name\" }}, got {:?}",
                other
            ),
        }
    }

    // variant-routing (exact): an empty top-level releases array yields zero parsed releases,
    // so the latest-release lookup finds nothing and must surface as EXACTLY
    // `NoReleaseFound { target: None }` -- the clean empty-listing negative, NOT a payload-field
    // failure. Pins the other side of the `NoReleaseFound | MissingAssetField` split.
    #[cfg(feature = "async")]
    #[tokio::test]
    async fn empty_array_routes_to_no_release_found_exactly() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "[]".to_string(),
            }]
        });
        let upd = gitea_update(&base, "0.1.0");
        let res = upd.get_latest_release_async().await;
        match res {
            Err(crate::errors::Error::NoReleaseFound { target }) => {
                assert_eq!(target, None, "empty listing carries no asset target");
            }
            other => panic!(
                "empty releases array must be Error::NoReleaseFound {{ target: None }}, got {:?}",
                other
            ),
        }
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_newer_releases_async_filters_to_newer_only() {
        // The single-page payload mixes releases newer than, equal to, and older than the current
        // version. `get_newer_releases_async` must keep only the strictly-newer ones, preserving
        // source order.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
            }]
        });
        let upd = gitea_update(&base, "1.0.0");
        let releases = upd.get_newer_releases_async().await.unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["2.0.0", "1.5.0"],
            "only releases strictly newer than the current version are kept, in order"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_newer_releases_async_empty_when_all_older_or_equal() {
        // When no release is newer than the current version, the filtered result is empty
        // (this is the "up to date" signal the higher-level async update flow relies on).
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v1.0.0", "v0.9.0"]),
            }]
        });
        let upd = gitea_update(&base, "1.0.0");
        let releases = upd.get_newer_releases_async().await.unwrap();
        assert!(
            releases.all().is_empty(),
            "no release newer than current => empty list, got {:?}",
            releases.all()
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_newer_releases_async_accumulates_across_pages_then_filters() {
        // Pagination must accumulate across pages: a newer release living on page 2 (reached via
        // the `Link: rel="next"` header) must be retained alongside page 1's. The listing is
        // newest-first, so page 1 carries the newest releases and page 2 the next-newest; the
        // early-stop only halts on a release NOT newer than current, which never happens here, so
        // page 2 is fetched and its release survives the strictly-newer filter.
        let base = stub(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v1/repos/o/r/releases?page=2")),
                    body: releases_json(&["v3.0.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v2.0.0"]),
                },
            ]
        });
        let upd = gitea_update(&base, "1.0.0");
        let releases = upd.get_newer_releases_async().await.unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["3.0.0", "2.0.0"],
            "the newer release on page 2 is reached and survives pagination + filtering"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_errors_on_empty_array() {
        // An empty releases array must `bail!` with `Error::Release`, not index out of bounds.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "[]".to_string(),
            }]
        });
        let upd = gitea_update(&base, "0.1.0");
        let res = upd.get_latest_release_async().await;
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::NoReleaseFound { .. }
                    | crate::errors::Error::MissingAssetField { .. })
            ),
            "empty releases array must surface as Error::Release, got {:?}",
            res
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_errors_on_non_array_payload() {
        // A non-array top-level payload (object) must hit the `as_array` guard and error.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "{}".to_string(),
            }]
        });
        let upd = gitea_update(&base, "0.1.0");
        let res = upd.get_latest_release_async().await;
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidResponse { .. })),
            "non-array payload must surface as Error::InvalidResponse, got {:?}",
            res
        );
    }

    // --- gap A: async backport regression ---------------------------------------------------------
    //
    // The sync regression tests (`early_stop_selects_same_release_as_a_full_walk` and
    // `get_newer_releases_continues_past_non_newer_releases_and_fetches_page_two`) pin the fix on
    // the sync path. This pins it on the async path: when page 1 has ONLY non-newer releases, the
    // async driver must still follow the Link header to page 2 and return the newer release there.
    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_backport_regression_page_one_all_non_newer_finds_newer_on_page_two() {
        let (base, captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v1/repos/o/r/releases?page=2")),
                    body: releases_json(&["v1.0.0", "v0.9.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v1.5.0"]),
                },
            ]
        });
        let upd = gitea_update(&base, "1.0.0");
        let releases = upd.get_newer_releases_async().await.unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["1.5.0"],
            "async: newer release on page 2 must be found when page 1 has only non-newer releases"
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "async: both pages must be fetched; the old early-stop bug fetched only one"
        );
    }

    // --- gap B: ReleaseList auth_token removed from struct -- verify token still reaches the wire -
    //
    // The dead `auth_token` field was removed from `ReleaseList`; auth now flows exclusively via
    // `request.auth_token`. This test verifies end-to-end: `ReleaseListBuilder::auth_token` must
    // cause `Authorization: token <secret>` to appear in the actual HTTP request.
    #[test]
    fn release_list_auth_token_transmitted_in_http_request() {
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
            auth_header.is_some_and(|l| l.contains("token secret")),
            "ReleaseList::fetch must transmit `Authorization: token secret`; \
             auth_token was removed from the ReleaseList struct and now only flows via \
             request.auth_token. header: {:?}",
            reqs[0]
                .lines()
                .find(|l| l.to_lowercase().starts_with("authorization:"))
        );
    }

    // --- gap C: newest_plan auth parameter removed -- verify token still reaches the wire --------
    //
    // `newest_plan` no longer takes an `auth_token` parameter; auth flows via
    // `common.request.auth_token -> apply_auth`. This verifies that `get_latest_release`
    // actually sends the Authorization header when an auth token is configured.
    #[test]
    fn get_latest_release_sync_transmits_auth_token_via_request_config() {
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.0.0"),
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
            auth_header.is_some_and(|l| l.contains("token mytoken")),
            "get_latest_release must transmit `Authorization: token mytoken`; \
             newest_plan's auth param was removed so the token must flow via \
             common.request -> apply_auth. header: {:?}",
            auth_header
        );
    }

    // --- gap D: sync path for non-array payload (async already covered) -------------------------
    #[test]
    fn sync_non_array_payload_routes_to_invalid_response_exactly() {
        // A top-level `{}` object cannot be deserialized as `Vec<ReleaseDto>`, so the parse branch
        // returns `Error::InvalidResponse` (a malformed listing body, distinct from a valid empty
        // `[]` which is `NoReleaseFound`). Sync mirror of the async test above.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "{}".to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        match upd.get_latest_release() {
            Err(crate::errors::Error::InvalidResponse { .. }) => {}
            other => panic!(
                "non-array payload must be Error::InvalidResponse, got {:?}",
                other
            ),
        }
    }

    // --- gap E: DTO field error paths -----------------------------------------------------------
    //
    // The populated-payload test covers the happy path. These pin the exact error variant and field
    // name for each of the four `ok_or(MissingAssetField)` calls in `into_release` / `into_asset`.

    #[test]
    fn dto_missing_created_at_surfaces_as_missing_asset_field() {
        // `created_at` absent -> `MissingAssetField { field: "created_at" }` (not tag_name, not assets).
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"[{"tag_name":"v1.0.0","assets":[]}]"#.to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        match upd.get_latest_release() {
            Err(crate::errors::Error::MissingAssetField { field }) => {
                assert_eq!(field, "created_at", "must name the absent field exactly");
            }
            other => panic!(
                "missing created_at must be MissingAssetField {{ field: \"created_at\" }}, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn dto_missing_assets_field_surfaces_as_missing_asset_field() {
        // `assets` absent -> `MissingAssetField { field: "assets" }`.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"[{"tag_name":"v1.0.0","created_at":"2020-01-01T00:00:00Z"}]"#.to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        match upd.get_latest_release() {
            Err(crate::errors::Error::MissingAssetField { field }) => {
                assert_eq!(field, "assets", "must name the absent field exactly");
            }
            other => panic!(
                "missing assets must be MissingAssetField {{ field: \"assets\" }}, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn dto_asset_missing_download_url_surfaces_as_missing_asset_field() {
        // `browser_download_url` is checked first in `into_asset`; its absence must surface before
        // the `name` check. A release with one asset that is missing the download URL is the
        // minimal case.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"[{"tag_name":"v1.0.0","created_at":"2020-01-01T00:00:00Z","assets":[{"name":"app.tar.gz"}]}]"#
                    .to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        match upd.get_latest_release() {
            Err(crate::errors::Error::MissingAssetField { field }) => {
                assert_eq!(
                    field, "browser_download_url",
                    "must name the absent asset field exactly"
                );
            }
            other => panic!(
                "missing browser_download_url must be MissingAssetField, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn dto_asset_missing_name_surfaces_as_missing_asset_field() {
        // `browser_download_url` is present but `name` is absent; the second field check fires.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"[{"tag_name":"v1.0.0","created_at":"2020-01-01T00:00:00Z","assets":[{"browser_download_url":"https://example.com/app"}]}]"#
                    .to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        match upd.get_latest_release() {
            Err(crate::errors::Error::MissingAssetField { field }) => {
                assert_eq!(field, "name", "must name the absent asset field exactly");
            }
            other => panic!(
                "missing asset name must be MissingAssetField {{ field: \"name\" }}, got {:?}",
                other
            ),
        }
    }

    // --- gap F: ReleaseList builder validates repo_owner and repo_name --------------------------
    //
    // `release_list_build_requires_url` only tests the `url` guard; these pin the other two
    // required fields, which have the same `MissingField` error path.
    #[test]
    fn release_list_build_requires_repo_owner_and_repo_name() {
        let res = super::ReleaseList::configure()
            .host("https://gitea.example.com")
            .repo_name("r")
            .build();
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::MissingField {
                    field: "repo_owner"
                })
            ),
            "missing repo_owner must surface as MissingField {{ field: \"repo_owner\" }}, got {:?}",
            res
        );

        let res = super::ReleaseList::configure()
            .host("https://gitea.example.com")
            .repo_owner("o")
            .build();
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::MissingField { field: "repo_name" })
            ),
            "missing repo_name must surface as MissingField {{ field: \"repo_name\" }}, got {:?}",
            res
        );
    }

    // --- gap G: name=null falls back to the raw tag_name ----------------------------------------
    //
    // `ReleaseDto::into_release` uses `self.name.unwrap_or_else(|| tag.clone())` where `tag` is the
    // raw `tag_name` value (e.g. "v1.2.3"). The version is separately stripped of a leading 'v'.
    // None of the existing tests supply `name: null`; this pins the fallback path.
    #[test]
    fn dto_null_name_falls_back_to_raw_tag_name() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"[{"tag_name":"v1.2.3","created_at":"2020-01-01T00:00:00Z","name":null,"assets":[]}]"#
                    .to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        let releases = upd.get_latest_release().unwrap();
        let rel = releases.latest().unwrap();
        assert_eq!(
            rel.version(),
            "1.2.3",
            "version() must strip the leading 'v'"
        );
        assert_eq!(
            rel.name(),
            "v1.2.3",
            "null name must fall back to the raw tag_name (the 'v' prefix is preserved in name)"
        );
    }

    // --- gap H: tag without a leading 'v' -------------------------------------------------------
    //
    // `tag.trim_start_matches('v')` is a no-op when the tag has no 'v' prefix; the version must
    // equal the tag verbatim. All existing tests use "v<ver>" tags.
    #[test]
    fn tag_without_leading_v_version_is_preserved() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"[{"tag_name":"1.2.3","created_at":"2020-01-01T00:00:00Z","name":"Release 1.2.3","assets":[]}]"#
                    .to_string(),
            }]
        });
        let upd = gitea_update_sync(&base, "0.1.0");
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(
            releases.latest().unwrap().version(),
            "1.2.3",
            "a tag without a leading 'v' must not be mangled by trim_start_matches"
        );
    }

    // --- gap I: filter_target actually filters releases ------------------------------------------
    //
    // `url_and_filter_target_setters_exist_on_release_list_builder` only verifies the builder
    // compiles and builds; it does not verify that `filter_target` actually drops releases.
    // This test makes two releases available, only one of which has an asset matching the target.
    #[test]
    fn filter_target_drops_releases_without_matching_asset() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: concat!(
                    r#"[{"tag_name":"v2.0.0","created_at":"2020-01-01T00:00:00Z","name":"v2.0.0","assets":[{"name":"app-x86_64-linux.tar.gz","browser_download_url":"https://example.com/2.0.0"}]},"#,
                    r#"{"tag_name":"v1.0.0","created_at":"2019-01-01T00:00:00Z","name":"v1.0.0","assets":[{"name":"app-windows.zip","browser_download_url":"https://example.com/1.0.0"}]}]"#
                ).to_string(),
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
        assert_eq!(
            releases.len(),
            1,
            "filter_target must drop releases with no matching asset; \
             only the x86_64-linux release must survive"
        );
        assert_eq!(
            releases[0].version(),
            "2.0.0",
            "the surviving release must be the one whose asset name contains the target"
        );
    }
}

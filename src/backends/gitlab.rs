/*!
Gitlab releases
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

/// GitLab release-asset link JSON shape (assets live under `assets.links`). Private DTO converted
/// into the public [`ReleaseAsset`]; keeping it private keeps `Deserialize` out of `ReleaseAsset`'s
/// public API.
#[derive(Deserialize)]
struct AssetDto {
    name: Option<String>,
    url: Option<String>,
}

impl AssetDto {
    fn into_asset(self) -> Result<ReleaseAsset> {
        let download_url = self.url.ok_or(Error::MissingAssetField { field: "url" })?;
        let name = self
            .name
            .ok_or(Error::MissingAssetField { field: "name" })?;
        Ok(ReleaseAsset::new(name, download_url))
    }
}

/// GitLab `assets` object wrapping the `links` array.
#[derive(Deserialize, Default)]
struct AssetsDto {
    links: Option<Vec<AssetDto>>,
}

/// GitLab release JSON shape (note: body is `description`, assets are nested under `assets.links`).
/// Private DTO deserialized directly from the response bytes, then converted into the public
/// [`Release`].
#[derive(Deserialize)]
struct ReleaseDto {
    tag_name: Option<String>,
    created_at: Option<String>,
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    assets: AssetsDto,
}

impl ReleaseDto {
    fn into_release(self) -> Result<Release> {
        let tag = self
            .tag_name
            .ok_or(Error::MissingAssetField { field: "tag_name" })?;
        let date = self.created_at.ok_or(Error::MissingAssetField {
            field: "created_at",
        })?;
        let links = self.assets.links.ok_or(Error::MissingAssetField {
            field: "assets.links",
        })?;
        let name = self.name.unwrap_or_else(|| tag.clone());
        let assets = links
            .into_iter()
            .map(AssetDto::into_asset)
            .collect::<Result<Vec<ReleaseAsset>>>()?;
        let mut builder = Release::builder();
        builder
            .name(name)
            .version(tag.trim_start_matches('v').to_owned())
            .date(date)
            .assets(assets);
        if let Some(body) = self.description {
            builder.body(body);
        }
        builder.build()
    }
}

/// `ReleaseList` Builder
#[derive(Clone, Debug)]
#[must_use]
pub struct ReleaseListBuilder {
    host: String,
    repo_owner: Option<String>,
    repo_name: Option<String>,
    target: Option<String>,
    auth_token: Option<String>,
    request: RequestConfig,
}
impl ReleaseListBuilder {
    /// Set the base URL of the GitLab instance, e.g. `https://gitlab.com`. Defaults to
    /// `https://gitlab.com`.
    ///
    /// Pass the instance host only (scheme + host, no trailing slash); the crate appends the
    /// `/api/v4/...` path itself. Do not include `/api/v4`.
    pub fn host(&mut self, url: impl Into<String>) -> &mut Self {
        self.host = url.into();
        self
    }

    /// Required. Set the repo owner, used to build a gitlab api url
    pub fn repo_owner(&mut self, owner: impl Into<String>) -> &mut Self {
        self.repo_owner = Some(owner.into());
        self
    }

    /// Required. Set the repo name, used to build a gitlab api url
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

    /// Set the authorization token, used in requests to the gitlab api url
    ///
    /// This is to support private repos where you need a Gitlab auth token.
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
        // Thread the auth token + gitlab's `Bearer` scheme into the request so the shared
        // `apply_auth` applies it on the listing path (honoring a user override).
        let mut request = self.request.clone();
        request.auth_scheme = crate::backends::common::AuthScheme::Bearer;
        request.auth_token = self.auth_token.clone();
        request.auth_base_host = crate::backends::common::host_of(&self.host);
        request.build_client();
        request.check()?;
        Ok(ReleaseList {
            host: self.host.clone(),
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
            auth_token: self.auth_token.clone(),
            request,
        })
    }
}

/// `ReleaseList` provides a builder api for querying a Gitlab repo,
/// returning a `Vec` of available `Release`s
#[derive(Clone, Debug)]
pub struct ReleaseList {
    host: String,
    repo_owner: String,
    repo_name: String,
    target: Option<String>,
    auth_token: Option<String>,
    request: RequestConfig,
}
impl ReleaseList {
    /// Initialize a ReleaseListBuilder
    pub fn configure() -> ReleaseListBuilder {
        ReleaseListBuilder {
            host: String::from("https://gitlab.com"),
            repo_owner: None,
            repo_name: None,
            target: None,
            auth_token: None,
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
            "{}/api/v4/projects/{}%2F{}/releases",
            self.host,
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        );
        // An unfiltered listing must walk ALL pages: `stop_at = None`.
        let releases = run_paginated(
            releases_plan(&api_url, self.auth_token.as_deref(), None)?,
            &self.request,
        )?;
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
            "{}/api/v4/projects/{}%2F{}/releases",
            self.host,
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        );
        // An unfiltered listing must walk ALL pages: `stop_at = None`.
        let releases = crate::backends::run_paginated_async(
            releases_plan(&api_url, self.auth_token.as_deref(), None)?,
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

/// `gitlab::Update` builder
///
/// Configure download and installation from
/// `https://gitlab.com/api/v4/projects/<repo_owner>%2F<repo_name>/releases`
#[derive(Clone, Debug)]
#[must_use]
pub struct UpdateBuilder {
    host: String,
    repo_owner: Option<String>,
    repo_name: Option<String>,
    common: CommonBuilderConfig,
}

impl UpdateBuilder {
    /// Initialize a new builder
    pub fn new() -> Self {
        Default::default()
    }

    /// Set the base URL of the GitLab instance, e.g. `https://gitlab.com`. Defaults to
    /// `https://gitlab.com`.
    ///
    /// Pass the instance host only (scheme + host, no trailing slash); the crate appends the
    /// `/api/v4/...` path itself. Do not include `/api/v4`.
    pub fn host(&mut self, url: impl Into<String>) -> &mut Self {
        self.host = url.into();
        self
    }

    /// Required. Set the repo owner, used to build a gitlab api url
    pub fn repo_owner(&mut self, owner: impl Into<String>) -> &mut Self {
        self.repo_owner = Some(owner.into());
        self
    }

    /// Required. Set the repo name, used to build a gitlab api url
    pub fn repo_name(&mut self, name: impl Into<String>) -> &mut Self {
        self.repo_name = Some(name.into());
        self
    }

    impl_common_builder_setters!();

    fn build_update(&self) -> Result<Update> {
        Ok(Update {
            host: self.host.to_owned(),
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
                // gitlab authenticates with the `Bearer` scheme; set it before resolving so the
                // shared `apply_auth` renders `Bearer <token>` on both listing and download.
                let mut common = self.common.clone();
                common.auth_scheme = crate::backends::common::AuthScheme::Bearer;
                let mut resolved = common.build()?;
                // Only the gitlab host receives the token; a server-supplied external asset link
                // (gitlab allows arbitrary asset URLs) does not.
                resolved.request.auth_base_host = crate::backends::common::host_of(&self.host);
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

    /// Confirm config and create a ready-to-use `Update` for the async API (`update_async`).
    ///
    /// Unlike [`build`](Self::build) this returns the concrete `Update` (not a
    /// `Box<dyn ReleaseUpdate>`) so the inherent `*_async` methods are reachable.
    #[cfg(feature = "async")]
    pub fn build_async(&self) -> Result<Update> {
        self.build_update()
    }
}

/// Updates to a specified or latest release distributed via Gitlab
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
            "{}/api/v4/projects/{}%2F{}/releases",
            self.host,
            urlencoding::encode(&self.repo_owner),
            urlencoding::encode(&self.repo_name)
        )
    }
}

impl crate::update::sealed::Sealed for Update {}

impl Update {
    /// The single-release-by-tag URL: `.../releases/{ver}`.
    fn tag_url(&self, ver: &str) -> String {
        format!("{}/{}", self.releases_url(), urlencoding::encode(ver))
    }
}

impl ReleaseUpdate for Update {
    fn get_latest_release(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = run_paginated(
            newest_plan(&self.releases_url(), self.common.auth_token.as_deref())?,
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
                self.common.auth_token.as_deref(),
                Some(&current_version),
            )?,
            &self.common.request,
        )?;
        Ok(Releases::new(releases, current_version))
    }

    fn get_release_version(&self, ver: &str) -> Result<Release> {
        let releases = run_paginated(
            single_plan(self.tag_url(ver), self.common.auth_token.as_deref())?,
            &self.common.request,
        )?;
        releases
            .into_iter()
            .next()
            .ok_or_else(|| Error::NoReleaseFound { target: None })
    }
}

impl_sync_update_verbs!(Update);

impl_update_config_accessors!(Update, {
    fn api_headers(&self, auth_token: Option<&str>) -> Result<header::HeaderMap> {
        api_headers(auth_token)
    }
});

impl Default for UpdateBuilder {
    fn default() -> Self {
        Self {
            host: String::from("https://gitlab.com"),
            repo_owner: None,
            repo_name: None,
            common: CommonBuilderConfig::default(),
        }
    }
}

/// Transport-free plan to fetch the paginated `releases` array (GitLab format), parsing each page
/// via the private `ReleaseDto` and following `Link: rel="next"`. `stop_at` filters per-item
/// (releases not strictly newer are omitted but pagination continues); when `None` all pages are
/// walked unfiltered.
fn releases_plan(
    base_url: &str,
    auth_token: Option<&str>,
    stop_at: Option<&str>,
) -> Result<PageRequest<Release>> {
    let headers = api_headers(auth_token)?;
    let auth = auth_token.map(str::to_owned);
    let stop_at = stop_at.map(str::to_owned);
    Ok(release_array_page(
        first_page_url(base_url),
        headers,
        auth,
        stop_at,
    ))
}

fn release_array_page(
    url: String,
    headers: HeaderMap,
    auth: Option<String>,
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
                let release = dto.into_release()?;
                // Skip releases not strictly newer than the current version, but do NOT stop
                // pagination. A backport release (older semver, newer creation date) must not
                // halt the walk; a genuinely newer release on a later page must still be found.
                if let Some(ref current) = stop_at {
                    if !bump_is_greater(current, release.version()).unwrap_or(false) {
                        continue;
                    }
                }
                items.push(release);
            }
            let next = next_link(resp_headers).map(|next_url| {
                release_array_page(
                    next_url,
                    api_headers(auth.as_deref()).unwrap_or_default(),
                    auth.clone(),
                    stop_at.clone(),
                )
            });
            Ok(Page {
                items,
                next,
                stop: false,
            })
        }),
    }
}

/// Transport-free plan for the newest release: GitLab has no `/releases/latest`, so the listing's
/// first element (newest-first order) is "latest". Fetches just the first page (no pagination).
fn newest_plan(base_url: &str, auth_token: Option<&str>) -> Result<PageRequest<Release>> {
    let headers = api_headers(auth_token)?;
    Ok(PageRequest {
        url: first_page_url(base_url),
        headers,
        parse: Box::new(|body, _resp_headers| {
            let dtos: Vec<ReleaseDto> =
                serde_json::from_slice(body).map_err(|e| Error::InvalidResponse {
                    source: Box::new(e),
                })?;
            let first = dtos
                .into_iter()
                .next()
                .ok_or_else(|| Error::NoReleaseFound { target: None })?;
            Ok(Page::last(vec![first.into_release()?]))
        }),
    })
}

/// Transport-free plan to fetch a single release *object* (the `.../releases/{ver}` endpoint).
fn single_plan(url: String, auth_token: Option<&str>) -> Result<PageRequest<Release>> {
    let headers = api_headers(auth_token)?;
    Ok(PageRequest {
        url,
        headers,
        parse: Box::new(|body, _resp_headers| {
            let dto: ReleaseDto = serde_json::from_slice(body)?;
            Ok(Page::last(vec![dto.into_release()?]))
        }),
    })
}

#[cfg(feature = "async")]
impl crate::update::AsyncReleaseUpdate for Update {
    async fn get_latest_release_async(&self) -> Result<Releases> {
        use crate::backends::run_paginated_async;
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = run_paginated_async(
            newest_plan(&self.releases_url(), self.common.auth_token.as_deref())?,
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
                self.common.auth_token.as_deref(),
                Some(&current_version),
            )?,
            &self.common.request,
        )
        .await?;
        Ok(Releases::new(releases, current_version))
    }

    async fn get_release_version_async(&self, ver: &str) -> Result<Release> {
        use crate::backends::run_paginated_async;
        let releases = run_paginated_async(
            single_plan(self.tag_url(ver), self.common.auth_token.as_deref())?,
            &self.common.request,
        )
        .await?;
        releases
            .into_iter()
            .next()
            .ok_or_else(|| Error::NoReleaseFound { target: None })
    }
}

/// Build gitlab's base request headers (its User-Agent). The Authorization header is applied
/// centrally by the shared [`apply_auth`](crate::backends::common::RequestConfig::apply_auth) using
/// gitlab's `Bearer` scheme on both the listing and download paths, honoring a user override.
fn api_headers(_auth_token: Option<&str>) -> Result<header::HeaderMap> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        "rust-reqwest/self-update"
            .parse()
            .expect("gitlab invalid user-agent"),
    );
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::Update;
    use crate::update::UpdateConfig;

    #[cfg(feature = "async")]
    use crate::update::AsyncReleaseUpdate;

    /// Async test wrapper over `releases_plan` + the async driver (unfiltered, all pages).
    #[cfg(feature = "async")]
    async fn fetch_all_releases_async(
        base_url: &str,
        auth_token: Option<&str>,
        req: &crate::backends::common::RequestConfig,
    ) -> crate::errors::Result<Vec<super::Release>> {
        crate::backends::run_paginated_async(super::releases_plan(base_url, auth_token, None)?, req)
            .await
    }

    // -----------------------------------------------------------------------
    // Shared loopback stub infrastructure (sync and async tests both use this)
    // -----------------------------------------------------------------------

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
                let _ = stream.read(&mut buf);
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

    /// Like [`stub`], but also captures each incoming raw request so tests can assert on what
    /// the client actually sent (e.g. to verify URL encoding in the request line).
    #[cfg_attr(not(feature = "async"), allow(dead_code))]
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

    /// A GitLab-format releases JSON array containing one release with the given tag.
    /// GitLab assets live under `assets.links` (not a bare `assets` array), and the
    /// body field is `description` (not `body`).
    fn release_json(tag: &str) -> String {
        format!(
            r#"[{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":{{"links":[]}},"description":null}}]"#
        )
    }

    /// A GitLab-format releases JSON array containing one release per entry in `tags`.
    fn releases_json(tags: &[&str]) -> String {
        let objs = tags
            .iter()
            .map(|tag| {
                format!(
                    r#"{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":{{"links":[]}},"description":null}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("[{objs}]")
    }

    /// A bare GitLab-format release object (not wrapped in an array). GitLab's
    /// `get_release_version[_async]` hits `.../releases/{ver}`, which returns a single object.
    fn release_obj_json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":{{"links":[]}},"description":null}}"#
        )
    }

    /// Convenience: build a `gitlab::Update` (concrete type) pointed at the loopback stub.
    /// Only available when the `async` feature is enabled (uses `build_async()`).
    #[cfg(feature = "async")]
    fn gitlab_update(base: &str, current_version: &str) -> Update {
        Update::configure()
            .host(base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version(current_version)
            .build_async()
            .unwrap()
    }

    /// Convenience: build a sync `Update` pointed at the loopback stub.
    /// Available under both sync transports (reqwest blocking and ureq).
    fn gl_update(base: &str, current_version: &str) -> Update {
        Update::configure()
            .host(base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version(current_version)
            .build()
            .unwrap()
    }

    // -----------------------------------------------------------------------
    // Async tests
    // -----------------------------------------------------------------------

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_parses_release() {
        // Drive `get_latest_release_async` against a loopback mock that returns a GitLab-format
        // releases array and assert the parsed version string.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.5.0"),
            }]
        });
        let upd = gitlab_update(&base, "0.1.0");

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
                    link: Some(format!("{base}/api/v4/projects/o%2Fr/releases?page=2")),
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
            &format!("{base}/api/v4/projects/o%2Fr/releases"),
            None,
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
    async fn is_update_available_async_true_when_latest_is_newer() {
        // D2 (async): the pre-check is now `get_latest_release_async().await?.is_update_available()`
        // on the returned `Releases`. It must report an available update when the backend's latest
        // release is newer than the current version, using only the listing request (no
        // download/install). github/gitea share the identical generated method, so covering one git
        // backend's async path exercises the shared code path.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.5.0"),
            }]
        });
        let upd = gitlab_update(&base, "0.1.0");
        assert!(
            upd.get_latest_release_async()
                .await
                .unwrap()
                .is_update_available()
                .unwrap(),
            "2.5.0 > 0.1.0 => async update available"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn is_update_available_async_false_when_latest_not_newer() {
        // D2 (async) complement: no update when current >= latest.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.5.0"),
            }]
        });
        let upd = gitlab_update(&base, "2.5.0");
        assert!(
            !upd.get_latest_release_async()
                .await
                .unwrap()
                .is_update_available()
                .unwrap(),
            "2.5.0 not newer than 2.5.0 => no async update"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn is_update_available_async_propagates_empty_array_error() {
        // D2 (async): an empty releases array yields no latest release; the error from
        // `get_latest_release_async` must propagate (rather than being swallowed into `Ok(false)`)
        // when the caller chains `.is_update_available()` — the error happens at the fetch.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "[]".to_string(),
            }]
        });
        let upd = gitlab_update(&base, "0.1.0");
        let res = upd.get_latest_release_async().await;
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::NoReleaseFound { .. }
                    | crate::errors::Error::MissingAssetField { .. })
            ),
            "an empty releases array must propagate as Error::Release out of \
             get_latest_release_async, got {:?}",
            res
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_release_version_async_parses_single_tag_object() {
        // `.../releases/{ver}` returns a single release *object* (not an array). The async
        // path must parse the bare object via `from_release_gitlab` and strip the leading `v`.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v4.2.1"),
            }]
        });

        let upd = gitlab_update(&base, "0.1.0");
        let rel = upd.get_release_version_async("v4.2.1").await.unwrap();
        assert_eq!(rel.version(), "4.2.1");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_newer_releases_async_filters_to_newer_only() {
        // The payload mixes releases newer than, equal to, and older than the current version.
        // `get_newer_releases_async` must keep only strictly-newer ones, preserving order.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
            }]
        });

        let upd = gitlab_update(&base, "1.0.0");
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
        // When no release is newer than the current version, the result is empty.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v1.0.0", "v0.9.0"]),
            }]
        });

        let upd = gitlab_update(&base, "1.0.0");
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
        // Pagination must accumulate across pages: a newer release on page 2 (reached via a
        // `Link: rel="next"` header) must be retained alongside page 1's. The listing is
        // newest-first, so page 1 carries the newest releases and page 2 the next-newest; the
        // early-stop only halts on a release NOT newer than current, which never happens here, so
        // page 2 is fetched and its release survives the strictly-newer filter.
        let base = stub(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v4/projects/o%2Fr/releases?page=2")),
                    body: releases_json(&["v3.0.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v2.0.0"]),
                },
            ]
        });

        let upd = gitlab_update(&base, "1.0.0");
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
        // An empty releases array must error with `Error::Release`, not index out of bounds.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "[]".to_string(),
            }]
        });

        let upd = gitlab_update(&base, "0.1.0");
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
        // A non-array top-level payload must hit the `as_array` guard and error.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "{}".to_string(),
            }]
        });

        let upd = gitlab_update(&base, "0.1.0");
        let res = upd.get_latest_release_async().await;
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidResponse { .. })),
            "non-array payload must surface as Error::InvalidResponse, got {:?}",
            res
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_release_version_async_errors_on_missing_tag_name() {
        // A malformed object (no `tag_name`) must surface as `Error::Release`, not panic.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"{"created_at":"2020-01-01T00:00:00Z","assets":{"links":[]}}"#.to_string(),
            }]
        });

        let upd = gitlab_update(&base, "0.1.0");
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
    // accepts either via an `A | B` match; this pins the precise variant and the field name so a
    // regression that conflates a malformed-payload failure with the empty-listing variant (or
    // names the wrong field) is caught.
    #[cfg(feature = "async")]
    #[tokio::test]
    async fn missing_tag_name_routes_to_missing_asset_field_exactly() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"{"created_at":"2020-01-01T00:00:00Z","assets":{"links":[]}}"#.to_string(),
            }]
        });

        let upd = gitlab_update(&base, "0.1.0");
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

        let upd = gitlab_update(&base, "0.1.0");
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
    async fn releases_url_encodes_repo_owner_with_slash() {
        // `releases_url()` calls `urlencoding::encode` on `repo_owner`. A `repo_owner` that
        // contains a `/` (e.g. a subgroup path like "group/subgroup") must appear as `%2F` in
        // the request line seen by the server, not as a literal `/` that would create an extra
        // path segment.
        let (base, captured) = stub_capturing(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v1.0.0"),
            }]
        });

        let upd = Update::configure()
            .host(&base)
            .repo_owner("group/subgroup")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build_async()
            .unwrap();
        let _ = upd.get_latest_release_async().await.unwrap();
        let request = captured.lock().unwrap()[0].clone();
        // The request line (first line of the raw HTTP/1.1 request) must contain the
        // percent-encoded form of the slash.
        let first_line = request.lines().next().unwrap_or("");
        assert!(
            first_line.contains("%2F"),
            "repo_owner slash must be percent-encoded as %2F in the request path; got: {:?}",
            first_line
        );
        assert!(
            !first_line.contains("group/subgroup"),
            "literal slash in repo_owner must not appear in request path; got: {:?}",
            first_line
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_errors_on_non_2xx_status() {
        // The http client bails on any non-2xx status before the body is parsed. A 500 maps to
        // `Error::HttpStatus`. Drive the single-page `get_latest_release_async` against a 500 so
        // the status guard, not the JSON parser, is what fails.
        let base = stub(|_| {
            vec![Resp {
                status: "500 Internal Server Error",
                link: None,
                body: "boom".to_string(),
            }]
        });
        let upd = gitlab_update(&base, "0.1.0");
        let res = upd.get_latest_release_async().await;
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::HttpStatus { status: 500, .. })
            ),
            "non-2xx 500 on get_latest_release_async must surface as Error::HttpStatus(500), got {:?}",
            res
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_release_version_async_errors_on_non_2xx_status() {
        // Same non-2xx guard for the single-tag fetch path (`.../releases/{ver}`). A 404 maps to
        // `Error::NotFound`, not a parse attempt on the error body.
        let base = stub(|_| {
            vec![Resp {
                status: "404 Not Found",
                link: None,
                body: r#"{"message":"404 Not Found"}"#.to_string(),
            }]
        });
        let upd = gitlab_update(&base, "0.1.0");
        let res = upd.get_release_version_async("v9.9.9").await;
        assert!(
            matches!(res, Err(crate::errors::Error::NotFound { .. })),
            "non-2xx 404 on get_release_version_async must surface as Error::NotFound, got {:?}",
            res
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn fetch_all_releases_async_errors_on_non_2xx_status() {
        // The paginated async fetch path also enforces the non-2xx status guard on each page. A
        // 503 on the first page must abort the whole accumulation with `Error::HttpStatus`.
        let base = stub(|_| {
            vec![Resp {
                status: "503 Service Unavailable",
                link: None,
                body: "busy".to_string(),
            }]
        });
        let res = fetch_all_releases_async(
            &format!("{base}/api/v4/projects/o%2Fr/releases"),
            None,
            &crate::backends::common::RequestConfig::default(),
        )
        .await;
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::HttpStatus { status: 503, .. })
            ),
            "non-2xx 503 on fetch_all_releases_async must surface as Error::HttpStatus(503), got {:?}",
            res
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn fetch_all_releases_async_errors_when_body_is_not_an_array() {
        // Called directly with a 200 whose top-level JSON is an object, `fetch_all_releases_async`
        // must hit its own `as_array` guard ("No releases found") and surface `Error::Release`.
        // This is the paginated path's array check, distinct from `get_latest_release_async`'s.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "{}".to_string(),
            }]
        });
        let res = fetch_all_releases_async(
            &format!("{base}/api/v4/projects/o%2Fr/releases"),
            None,
            &crate::backends::common::RequestConfig::default(),
        )
        .await;
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidResponse { .. })),
            "non-array body on fetch_all_releases_async must surface as Error::InvalidResponse, got {:?}",
            res
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn fetch_all_releases_async_empty_array_is_ok_empty() {
        // Boundary: an empty top-level array is a *valid* (empty) page for the paginated fetch,
        // unlike `get_latest_release_async` where empty is an error. It must return an empty Vec,
        // not error.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "[]".to_string(),
            }]
        });
        let releases = fetch_all_releases_async(
            &format!("{base}/api/v4/projects/o%2Fr/releases"),
            None,
            &crate::backends::common::RequestConfig::default(),
        )
        .await
        .unwrap();
        assert!(
            releases.is_empty(),
            "empty releases array is a valid empty page, got {:?}",
            releases
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_errors_on_missing_assets_links() {
        // GitLab-specific parser path: `from_release_gitlab` requires `assets.links` to be an
        // array ("No assets found"). A release object lacking `assets.links` (but otherwise well
        // formed, with a valid `tag_name`/`created_at`) must surface as `Error::Release`, exercising
        // a different `from_release_gitlab` guard than the missing-`tag_name` case.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body:
                    r#"[{"tag_name":"v1.0.0","created_at":"2020-01-01T00:00:00Z","name":"v1.0.0"}]"#
                        .to_string(),
            }]
        });
        let upd = gitlab_update(&base, "0.1.0");
        let res = upd.get_latest_release_async().await;
        assert!(
            matches!(
                res,
                Err(crate::errors::Error::NoReleaseFound { .. }
                    | crate::errors::Error::MissingAssetField { .. })
            ),
            "missing assets.links must surface as Error::Release, got {:?}",
            res
        );
    }

    // -----------------------------------------------------------------------
    // Existing sync / builder tests
    // -----------------------------------------------------------------------

    #[test]
    fn url_and_filter_target_setters_exist_on_release_list_builder() {
        // The renamed `url` / `filter_target` setters must exist on the gitlab
        // `ReleaseListBuilder` and the builder must still build.
        let _list = super::ReleaseList::configure()
            .host("https://gitlab.example.com")
            .repo_owner("o")
            .repo_name("r")
            .filter_target("x86_64-unknown-linux-gnu")
            .build()
            .unwrap();
    }

    #[test]
    fn url_setter_exists_on_update_builder() {
        // The renamed `url` setter must exist on the gitlab `UpdateBuilder`.
        let _upd = Update::configure()
            .host("https://gitlab.example.com")
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
    }

    #[test]
    fn api_headers_override_uses_gitlab_user_agent() {
        // The `{api_headers}` override arm of `impl_update_config_accessors!` must wire gitlab's
        // custom `api_headers` (User-Agent), not the trait default (which sets no User-Agent). After
        // B5 the auth scheme/token is applied centrally by `apply_auth`, not baked here.
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
            "rust-reqwest/self-update"
        );
        assert!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .is_none(),
            "api_headers no longer bakes auth; apply_auth applies the Bearer scheme"
        );
    }

    // gitlab resolves to the `Bearer` scheme, applied by the shared `apply_auth` on the request
    // config consumed by BOTH the listing and download paths. A user `request_header(AUTHORIZATION)`
    // override wins.
    #[test]
    fn gitlab_bearer_scheme_applied_to_both_paths() {
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
        let mut headers = HeaderMap::new();
        upd.request_config()
            .apply_auth(
                "https://gitlab.com/api/v4/projects/o%2Fr/releases",
                &mut headers,
            )
            .unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer secret",
            "gitlab authenticates with the Bearer scheme"
        );

        // A user AUTHORIZATION override wins over the Bearer scheme.
        let upd = Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .auth_token("secret")
            .request_header(AUTHORIZATION, "token user-override")
            .build()
            .unwrap();
        let mut headers = upd.request_config().headers.clone();
        upd.request_config()
            .apply_auth(
                "https://gitlab.com/api/v4/projects/o%2Fr/releases",
                &mut headers,
            )
            .unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "token user-override",
            "a user AUTHORIZATION override must win over the Bearer scheme"
        );
    }

    #[test]
    fn release_list_build_surfaces_invalid_header() {
        // A bad header on the gitlab `ReleaseListBuilder` must fail at `build()` via
        // `request.check()` with `Error::Config`, not panic.
        let res = super::ReleaseList::configure()
            .repo_owner("o")
            .repo_name("r")
            .request_header("inva lid", "ok")
            .build();
        assert!(
            matches!(res, Err(crate::errors::Error::InvalidHeader { .. })),
            "invalid header must surface as Error::InvalidHeader from gitlab ReleaseList build()"
        );
    }

    #[test]
    fn update_build_surfaces_invalid_header() {
        // Same deferred-header check via `CommonBuilderConfig::build` on the gitlab UpdateBuilder.
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
        // `identifier` was previously missing from the gitlab builder.
        let upd = Update::configure()
            .repo_owner("owner")
            .repo_name("repo")
            .bin_name("my_bin")
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
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(upd.bin_path_in_archive(), expected);

        // An explicit `bin_path_in_archive` set before `bin_name` is NOT overwritten.
        let upd = Update::configure()
            .repo_owner("o")
            .repo_name("r")
            .bin_path_in_archive("custom/path")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(upd.bin_path_in_archive(), "custom/path");
    }

    // -----------------------------------------------------------------------
    // Sync loopback tests (plain #[test], no tokio, works under reqwest and ureq)
    // -----------------------------------------------------------------------

    #[test]
    fn get_latest_release_sync_parses_release() {
        // Drive `get_latest_release` (sync) against a loopback stub that returns a
        // GitLab-format releases array and assert the parsed version string.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.5.0"),
            }]
        });
        let upd = gl_update(&base, "0.1.0");
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(releases.latest().unwrap().version(), "2.5.0");
    }

    #[test]
    fn get_newer_releases_sync_filters_to_newer_only() {
        // The payload mixes releases newer than, equal to, and older than the current version.
        // `get_newer_releases` (sync) must keep only strictly-newer ones, preserving order.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
            }]
        });
        let upd = gl_update(&base, "1.0.0");
        let releases = upd.get_newer_releases().unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["2.0.0", "1.5.0"],
            "only releases strictly newer than the current version are kept, in order"
        );
    }

    #[test]
    fn get_release_version_sync_parses_single_tag_object() {
        // `.../releases/{ver}` returns a single release *object* (not an array). The sync
        // path must parse the bare object via `from_release_gitlab` and strip the leading `v`.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_obj_json("v4.2.1"),
            }]
        });
        let upd = gl_update(&base, "0.1.0");
        let rel = upd.get_release_version("v4.2.1").unwrap();
        assert_eq!(rel.version(), "4.2.1");
    }

    #[test]
    fn get_release_version_sync_errors_on_non_2xx_status() {
        // A 404 (the realistic "unknown tag" response from GitLab) must surface as an error,
        // not a parse attempt on the error body, under the sync transport.
        let base = stub(|_| {
            vec![Resp {
                status: "404 Not Found",
                link: None,
                body: r#"{"message":"404 Not Found"}"#.to_string(),
            }]
        });
        let upd = gl_update(&base, "0.1.0");
        let res = upd.get_release_version("v9.9.9");
        assert!(
            matches!(res, Err(crate::errors::Error::NotFound { .. })),
            "non-2xx 404 on get_release_version (sync) must surface as Error::NotFound, got {:?}",
            res
        );
    }

    #[test]
    fn is_update_available_sync_true_when_latest_is_newer() {
        // D1 (sync): the pre-check is now `get_newer_releases()?.is_update_available()`. The stub's
        // newest release is 2.5.0, so an update is available from 0.1.0.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.5.0"),
            }]
        });
        let upd = gl_update(&base, "0.1.0");
        assert!(
            upd.get_newer_releases()
                .unwrap()
                .is_update_available()
                .unwrap(),
            "2.5.0 > 0.1.0 => update available"
        );
    }

    #[test]
    fn is_update_available_sync_false_when_latest_not_newer() {
        // D1 complement: when the current version is at/above the latest release, no update is
        // available. `get_newer_releases` returns the single newer-filtered list (empty here);
        // current is 2.5.0, so nothing is newer.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: release_json("v2.5.0"),
            }]
        });
        let upd = gl_update(&base, "2.5.0");
        assert!(
            !upd.get_newer_releases()
                .unwrap()
                .is_update_available()
                .unwrap(),
            "2.5.0 not newer than 2.5.0 => no update"
        );
    }

    #[test]
    fn is_update_available_sync_propagates_non_2xx_error() {
        // D1 (sync): a backend HTTP failure (500) during the listing request must propagate out
        // of `get_newer_releases`, not be hidden as "no update available".
        let base = stub(|_| {
            vec![Resp {
                status: "500 Internal Server Error",
                link: None,
                body: r#"{"message":"boom"}"#.to_string(),
            }]
        });
        let upd = gl_update(&base, "0.1.0");
        let res = upd.get_newer_releases();
        assert!(
            res.is_err(),
            "a non-2xx listing response must propagate as an error out of \
             get_newer_releases, got {:?}",
            res
        );
    }

    #[test]
    fn get_latest_release_sync_errors_on_empty_array() {
        // An empty releases array must error, not index out of bounds, under the sync transport.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "[]".to_string(),
            }]
        });
        let upd = gl_update(&base, "0.1.0");
        let res = upd.get_latest_release();
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

    // --- gitlab git release-scan pagination (per-backend parser wiring) -----------------
    //
    // The gitlab parser (`release_array_page`) wires `stop_at` for per-item filtering without
    // halting pagination. These pin that wiring: non-newer releases are omitted but the driver
    // continues to subsequent pages, and the selection from a partial page must match a full walk.

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
                    link: Some(format!("{base}/api/v4/projects/o%2Fr/releases?page=2")),
                    body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v3.0.0"]),
                },
            ]
        });
        let upd = gl_update(&base, "1.0.0");
        let releases = upd.get_newer_releases().unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        // Non-newer items (v1.0.0, v0.9.0) are filtered out per-item; newer items from both
        // pages are kept. v3.0.0 from page 2 is present, proving pagination was not halted.
        assert_eq!(
            versions,
            vec!["2.0.0", "1.5.0", "3.0.0"],
            "non-newer items are filtered per-item; page 2 is still fetched and its newer release included"
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "non-newer releases must not halt pagination; both pages must be requested"
        );
    }

    #[test]
    fn get_newer_releases_finds_update_on_page_2_when_page_1_has_only_non_newer() {
        // Backport scenario: page 1 contains only a release that is NOT newer than the current
        // version (e.g. a backport patch with an older semver but a newer creation date).
        // The old version-based early-stop would have halted after page 1 and silently missed
        // the genuinely newer release on page 2. After the fix, pagination continues and v3.0.0
        // is correctly found.
        //
        // This test would FAIL on the unfixed code (early-stop discards page 2) and PASS on the
        // fixed code (per-item filter skips v0.5.0 but pagination continues to page 2).
        let (base, captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v4/projects/o%2Fr/releases?page=2")),
                    body: releases_json(&["v0.5.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v3.0.0"]),
                },
            ]
        });
        let upd = gl_update(&base, "1.0.0");
        let releases = upd.get_newer_releases().unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version()).collect();
        assert_eq!(
            versions,
            vec!["3.0.0"],
            "the newer release on page 2 must be found even when page 1 has only non-newer releases"
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "both pages must be fetched: non-newer releases must not halt pagination"
        );
    }

    #[test]
    fn early_stop_selects_same_release_as_a_full_walk() {
        // Selection parity: the early-stopped `get_newer_releases` must let the orchestrator pick
        // the SAME release a full unfiltered walk would, driven through `choose_latest_release`.
        let (base, _captured) = stub_capturing(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v4/projects/o%2Fr/releases?page=2")),
                    body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v0.5.0"]),
                },
            ]
        });
        let upd = gl_update(&base, "1.0.0");
        let early = upd.get_newer_releases().unwrap().into_vec();
        let early_choice =
            crate::update::testing::choose_latest_release_for_test(early, "1.0.0").unwrap();

        let full: Vec<_> = ["2.0.0", "1.5.0", "1.0.0", "0.9.0", "0.5.0"]
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
            "early-stop must select the same release as a full walk"
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
                    link: Some(format!("{base}/api/v4/projects/o%2Fr/releases?page=2")),
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
    // whose getters surface every field. The other gitlab parse tests use empty `assets.links`
    // and a null `description`; this pins the populated mapping that differs in shape from the
    // other forges (assets nested under `assets.links`, body in `description`).
    #[test]
    fn dto_parse_maps_populated_payload_through_getters() {
        let body = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"[{"tag_name":"v3.4.5","created_at":"2021-07-08T09:10:11Z","name":"My App 3.4.5","description":"the notes","assets":{"links":[{"name":"app-x86_64-linux.tar.gz","url":"https://gl.example/app-x86_64-linux.tar.gz"},{"name":"app-aarch64-linux.tar.gz","url":"https://gl.example/app-aarch64-linux.tar.gz"}]}}]"#
                    .to_string(),
            }]
        });
        let upd = gl_update(&body, "0.1.0");
        let releases = upd.get_latest_release().unwrap();
        let rel = releases.latest().unwrap();
        assert_eq!(rel.version(), "3.4.5", "leading `v` stripped from tag_name");
        assert_eq!(rel.name(), "My App 3.4.5", "name surfaces from `name`");
        assert_eq!(rel.date(), "2021-07-08T09:10:11Z", "date from `created_at`");
        assert_eq!(
            rel.body(),
            Some("the notes"),
            "body surfaces from gitlab's `description` field"
        );
        assert_eq!(rel.assets().len(), 2, "both `assets.links` entries parsed");
        assert_eq!(rel.assets()[0].name(), "app-x86_64-linux.tar.gz");
        assert_eq!(
            rel.assets()[0].download_url(),
            "https://gl.example/app-x86_64-linux.tar.gz",
            "asset download_url comes from the link `url` field"
        );
        assert_eq!(rel.assets()[1].name(), "app-aarch64-linux.tar.gz");
    }

    // --- the listing `Releases` from `ReleaseList::fetch` carries NO current
    // version, so `current_version()` is `None` and `is_update_available()` errors with EXACTLY
    // `MissingField { field: "current_version" }` rather than silently answering. `into_vec()`
    // still recovers the underlying release vec.
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
                Err(crate::errors::Error::MissingField {
                    field: "current_version"
                })
            ),
            "is_update_available() on a listing must error with MissingField, got {:?}",
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
    // `ReleaseList::fetch_async` yields the same bare listing (no current version, MissingField
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
                Err(crate::errors::Error::MissingField {
                    field: "current_version"
                })
            ),
            "is_update_available() on a listing must error with MissingField, got {:?}",
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

    // --- exact-variant routing on the sync transport (gitlab). The sibling
    // exact-variant tests are `#[cfg(feature = "async")]` only, so the ureq lane (no async) never
    // pins the precise variant. This pins it on whichever sync client is active: a release object
    // missing `tag_name` must surface as EXACTLY `MissingAssetField { field: "tag_name" }`.
    #[test]
    fn sync_missing_tag_name_routes_to_missing_asset_field_exactly() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: r#"{"created_at":"2020-01-01T00:00:00Z","name":"x","assets":{"links":[]}}"#
                    .to_string(),
            }]
        });
        let upd = gl_update(&base, "0.1.0");
        let res = upd.get_release_version("v1.0.0");
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

    // --- the other side of the sync-lane split (gitlab): an empty top-level releases
    // array surfaces as EXACTLY `NoReleaseFound { target: None }`, not a payload-field failure.
    #[test]
    fn sync_empty_array_routes_to_no_release_found_exactly() {
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: "[]".to_string(),
            }]
        });
        let upd = gl_update(&base, "0.1.0");
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
}

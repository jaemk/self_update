/*!
Gitlab releases
*/
use crate::http_client::{header, HttpResponse};

use crate::backends::common::{CommonBuilderConfig, CommonConfig, RequestConfig};
use crate::backends::{collect_paginated, first_page_url, next_link, send};
use crate::version::bump_is_greater;
use crate::{
    errors::*,
    update::{Release, ReleaseAsset, ReleaseUpdate, Releases},
};

impl ReleaseAsset {
    /// Parse a release-asset json object
    ///
    /// Errors:
    ///     * Missing required name & download-url keys
    fn from_asset_gitlab(asset: &serde_json::Value) -> Result<ReleaseAsset> {
        let download_url = asset["url"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Asset missing `url`"))?;
        let name = asset["name"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Asset missing `name`"))?;
        Ok(ReleaseAsset {
            download_url: download_url.to_owned(),
            name: name.to_owned(),
        })
    }
}

impl Release {
    fn from_release_gitlab(release: &serde_json::Value) -> Result<Release> {
        let tag = release["tag_name"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `tag_name`"))?;
        let date = release["created_at"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `created_at`"))?;
        let name = release["name"].as_str().unwrap_or(tag);
        let assets = release["assets"]["links"]
            .as_array()
            .ok_or_else(|| format_err!(Error::Release, "No assets found"))?;
        let body = release["description"].as_str().map(String::from);
        let assets = assets
            .iter()
            .map(ReleaseAsset::from_asset_gitlab)
            .collect::<Result<Vec<ReleaseAsset>>>()?;
        Ok(Release {
            name: name.to_owned(),
            version: tag.trim_start_matches('v').to_owned(),
            date: date.to_owned(),
            body,
            assets,
        })
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
    pub fn url(&mut self, url: impl Into<String>) -> &mut Self {
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
        self.request.check()?;
        Ok(ReleaseList {
            host: self.host.clone(),
            repo_owner: if let Some(ref owner) = self.repo_owner {
                owner.to_owned()
            } else {
                bail!(
                    Error::Config,
                    "`repo_owner` required (call `.repo_owner(...)`)"
                )
            },
            repo_name: if let Some(ref name) = self.repo_name {
                name.to_owned()
            } else {
                bail!(
                    Error::Config,
                    "`repo_name` required (call `.repo_name(...)`)"
                )
            },
            target: self.target.clone(),
            auth_token: self.auth_token.clone(),
            request: self.request.clone(),
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

    /// Retrieve a list of `Release`s.
    /// If specified, filter for those containing a specified `target`
    pub fn fetch(&self) -> Result<Vec<Release>> {
        let api_url = format!(
            "{}/api/v4/projects/{}%2F{}/releases",
            self.host,
            urlencoding::encode(&self.repo_owner),
            self.repo_name
        );
        let releases = fetch_all_releases(&api_url, self.auth_token.as_deref(), &self.request)?;
        let releases = match self.target {
            None => releases,
            Some(ref target) => releases
                .into_iter()
                .filter(|r| r.has_target_asset(target))
                .collect::<Vec<_>>(),
        };
        Ok(releases)
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
    pub fn url(&mut self, url: impl Into<String>) -> &mut Self {
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
                bail!(
                    Error::Config,
                    "`repo_owner` required (call `.repo_owner(...)`)"
                )
            },
            repo_name: if let Some(ref name) = self.repo_name {
                name.to_owned()
            } else {
                bail!(
                    Error::Config,
                    "`repo_name` required (call `.repo_name(...)`)"
                )
            },
            common: self.common.build()?,
        })
    }

    /// Confirm config and create a ready-to-use `Update`
    ///
    /// * Errors:
    ///     * Config - Invalid `Update` configuration
    pub fn build(&self) -> Result<Box<dyn ReleaseUpdate>> {
        Ok(Box::new(self.build_update()?))
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

#[cfg(feature = "async")]
impl Update {
    impl_async_update_methods!();
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
            self.repo_name
        )
    }
}

impl crate::update::sealed::Sealed for Update {}

impl Update {
    /// Fetch and parse the single newest release (network helper; returns a bare `Release`).
    fn fetch_latest_release(&self) -> Result<Release> {
        let api_url = self.releases_url();
        let resp = send(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )?;
        let json = resp.json::<serde_json::Value>()?;
        let releases = json
            .as_array()
            .ok_or_else(|| format_err!(Error::Release, "no releases found"))?;
        if releases.is_empty() {
            bail!(Error::Release, "no releases found");
        }
        // Unlike github (which hits a dedicated `/releases/latest` endpoint), gitlab has no such
        // endpoint, so "newest" is `releases[0]` and relies on the list endpoint's default
        // descending (newest-first) order.
        Release::from_release_gitlab(&releases[0])
    }

    /// Fetch the full (paginated) release list, keeping only those newer than `current_version`
    /// (network helper; returns a bare `Vec<Release>`). `current_version` still bounds the filter.
    fn fetch_newer_releases(&self, current_version: &str) -> Result<Vec<Release>> {
        let api_url = self.releases_url();
        let releases = fetch_all_releases(
            &api_url,
            self.common.auth_token.as_deref(),
            &self.common.request,
        )?;
        Ok(releases
            .into_iter()
            .filter(|r| bump_is_greater(current_version, &r.version).unwrap_or(false))
            .collect())
    }
}

impl ReleaseUpdate for Update {
    fn get_latest_release(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let release = self.fetch_latest_release()?;
        Ok(Releases::new(vec![release], current_version))
    }

    fn get_latest_releases(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = self.fetch_newer_releases(&current_version)?;
        Ok(Releases::new(releases, current_version))
    }

    fn get_release_version(&self, ver: &str) -> Result<Release> {
        let api_url = format!("{}/{}", self.releases_url(), urlencoding::encode(ver));
        let resp = send(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )?;
        let json = resp.json::<serde_json::Value>()?;
        Release::from_release_gitlab(&json)
    }
}

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

/// Fetch every release from `base_url`, following GitLab's `Link: rel="next"` pagination.
fn fetch_all_releases(
    base_url: &str,
    auth_token: Option<&str>,
    req: &RequestConfig,
) -> Result<Vec<Release>> {
    collect_paginated(&first_page_url(base_url), |url| {
        let resp = send(url, api_headers(auth_token)?, req)?;
        let headers = resp.headers().clone();
        let releases = resp
            .json::<serde_json::Value>()?
            .as_array()
            .ok_or_else(|| format_err!(Error::Release, "No releases found"))?
            .iter()
            .map(Release::from_release_gitlab)
            .collect::<Result<Vec<Release>>>()?;
        Ok((releases, next_link(&headers)))
    })
}

/// Async sibling of [`fetch_all_releases`], following GitLab's `Link: rel="next"` pagination with
/// the async transport. Reuses the same [`Release::from_release_gitlab`] parser.
#[cfg(feature = "async")]
async fn fetch_all_releases_async(
    base_url: &str,
    auth_token: Option<&str>,
    req: &RequestConfig,
) -> Result<Vec<Release>> {
    use crate::backends::{collect_paginated_async, send_async};
    let auth = auth_token.map(str::to_owned);
    collect_paginated_async(&first_page_url(base_url), |url| {
        let auth = auth.clone();
        let req = req.clone();
        async move {
            let resp = send_async(&url, api_headers(auth.as_deref())?, &req).await?;
            let headers = resp.headers().clone();
            let releases = resp
                .json::<serde_json::Value>()
                .await?
                .as_array()
                .ok_or_else(|| format_err!(Error::Release, "No releases found"))?
                .iter()
                .map(Release::from_release_gitlab)
                .collect::<Result<Vec<Release>>>()?;
            Ok((releases, next_link(&headers)))
        }
    })
    .await
}

#[cfg(feature = "async")]
impl crate::update::AsyncFetch for Update {
    async fn get_latest_release_async(&self) -> Result<Releases> {
        use crate::backends::send_async;
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let api_url = self.releases_url();
        let resp = send_async(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )
        .await?;
        let json = resp.json::<serde_json::Value>().await?;
        let releases = json
            .as_array()
            .ok_or_else(|| format_err!(Error::Release, "no releases found"))?;
        if releases.is_empty() {
            bail!(Error::Release, "no releases found");
        }
        let release = Release::from_release_gitlab(&releases[0])?;
        Ok(Releases::new(vec![release], current_version))
    }

    async fn get_latest_releases_async(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let api_url = self.releases_url();
        let releases = fetch_all_releases_async(
            &api_url,
            self.common.auth_token.as_deref(),
            &self.common.request,
        )
        .await?;
        let releases = releases
            .into_iter()
            .filter(|r| bump_is_greater(&current_version, &r.version).unwrap_or(false))
            .collect();
        Ok(Releases::new(releases, current_version))
    }

    async fn get_release_version_async(&self, ver: &str) -> Result<Release> {
        use crate::backends::send_async;
        let api_url = format!("{}/{}", self.releases_url(), urlencoding::encode(ver));
        let resp = send_async(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )
        .await?;
        let json = resp.json::<serde_json::Value>().await?;
        Release::from_release_gitlab(&json)
    }
}

fn api_headers(auth_token: Option<&str>) -> Result<header::HeaderMap> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        "rust-reqwest/self-update"
            .parse()
            .expect("gitlab invalid user-agent"),
    );

    if let Some(token) = auth_token {
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", token)
                .parse()
                .map_err(|err| Error::Config(format!("Failed to parse auth token: {}", err)))?,
        );
    };

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::Update;

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
            .url(base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version(current_version)
            .build_async()
            .unwrap()
    }

    /// Convenience: build a `Box<dyn ReleaseUpdate>` pointed at the loopback stub.
    /// Available under both sync transports (reqwest blocking and ureq).
    fn gl_update(base: &str, current_version: &str) -> Box<dyn crate::update::ReleaseUpdate> {
        Update::configure()
            .url(base)
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
        assert_eq!(releases.latest().unwrap().version, "2.5.0");
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
        let releases = super::fetch_all_releases_async(
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
        assert_eq!(releases[0].version, "1.0.0");
        assert_eq!(releases[1].version, "0.9.0");
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
            matches!(res, Err(crate::errors::Error::Release(_))),
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
        assert_eq!(rel.version, "4.2.1");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_releases_async_filters_to_newer_only() {
        // The payload mixes releases newer than, equal to, and older than the current version.
        // `get_latest_releases_async` must keep only strictly-newer ones, preserving order.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
            }]
        });

        let upd = gitlab_update(&base, "1.0.0");
        let releases = upd.get_latest_releases_async().await.unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version.as_str()).collect();
        assert_eq!(
            versions,
            vec!["2.0.0", "1.5.0"],
            "only releases strictly newer than the current version are kept, in order"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_releases_async_empty_when_all_older_or_equal() {
        // When no release is newer than the current version, the result is empty.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v1.0.0", "v0.9.0"]),
            }]
        });

        let upd = gitlab_update(&base, "1.0.0");
        let releases = upd.get_latest_releases_async().await.unwrap();
        assert!(
            releases.all().is_empty(),
            "no release newer than current => empty list, got {:?}",
            releases.all()
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_releases_async_accumulates_across_pages_then_filters() {
        // Filtering must happen *after* pagination: a newer release on page 2 (reached via
        // a `Link: rel="next"` header) must still be retained.
        let base = stub(|base| {
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

        let upd = gitlab_update(&base, "1.0.0");
        let releases = upd.get_latest_releases_async().await.unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version.as_str()).collect();
        assert_eq!(
            versions,
            vec!["3.0.0"],
            "the newer release on page 2 survives pagination + filtering"
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
            matches!(res, Err(crate::errors::Error::Release(_))),
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
            matches!(res, Err(crate::errors::Error::Release(_))),
            "non-array payload must surface as Error::Release, got {:?}",
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
            matches!(res, Err(crate::errors::Error::Release(_))),
            "missing tag_name must surface as Error::Release, got {:?}",
            res
        );
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
            .url(&base)
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
            matches!(res, Err(crate::errors::Error::HttpStatus { status: 500, .. })),
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
        let res = super::fetch_all_releases_async(
            &format!("{base}/api/v4/projects/o%2Fr/releases"),
            None,
            &crate::backends::common::RequestConfig::default(),
        )
        .await;
        assert!(
            matches!(res, Err(crate::errors::Error::HttpStatus { status: 503, .. })),
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
        let res = super::fetch_all_releases_async(
            &format!("{base}/api/v4/projects/o%2Fr/releases"),
            None,
            &crate::backends::common::RequestConfig::default(),
        )
        .await;
        assert!(
            matches!(res, Err(crate::errors::Error::Release(_))),
            "non-array body on fetch_all_releases_async must surface as Error::Release, got {:?}",
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
        let releases = super::fetch_all_releases_async(
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
            matches!(res, Err(crate::errors::Error::Release(_))),
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
            .url("https://gitlab.example.com")
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
            .url("https://gitlab.example.com")
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
    }

    #[test]
    fn api_headers_override_uses_gitlab_user_agent_and_bearer_scheme() {
        // The `{api_headers}` override arm of `impl_update_config_accessors!` must wire gitlab's
        // custom `api_headers` (User-Agent + `Bearer` auth scheme), not the trait default (which
        // sets no User-Agent and a `token` scheme).
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
        assert_eq!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer secret",
            "gitlab authenticates with the Bearer scheme"
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
            matches!(res, Err(crate::errors::Error::Config(_))),
            "invalid header must surface as Error::Config from gitlab ReleaseList build()"
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
        assert!(matches!(res, Err(crate::errors::Error::Config(_))));
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
        assert_eq!(releases.latest().unwrap().version, "2.5.0");
    }

    #[test]
    fn get_latest_releases_sync_filters_to_newer_only() {
        // The payload mixes releases newer than, equal to, and older than the current version.
        // `get_latest_releases` (sync) must keep only strictly-newer ones, preserving order.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
            }]
        });
        let upd = gl_update(&base, "1.0.0");
        let releases = upd.get_latest_releases().unwrap();
        let versions: Vec<&str> = releases.all().iter().map(|r| r.version.as_str()).collect();
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
        assert_eq!(rel.version, "4.2.1");
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
        // D1 (sync): the pre-check is now `get_latest_releases()?.is_update_available()`. The stub's
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
            upd.get_latest_releases()
                .unwrap()
                .is_update_available()
                .unwrap(),
            "2.5.0 > 0.1.0 => update available"
        );
    }

    #[test]
    fn is_update_available_sync_false_when_latest_not_newer() {
        // D1 complement: when the current version is at/above the latest release, no update is
        // available. `get_latest_releases` returns the single newer-filtered list (empty here);
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
            !upd.get_latest_releases()
                .unwrap()
                .is_update_available()
                .unwrap(),
            "2.5.0 not newer than 2.5.0 => no update"
        );
    }

    #[test]
    fn is_update_available_sync_propagates_non_2xx_error() {
        // D1 (sync): a backend HTTP failure (500) during the listing request must propagate out
        // of `get_latest_releases`, not be hidden as "no update available".
        let base = stub(|_| {
            vec![Resp {
                status: "500 Internal Server Error",
                link: None,
                body: r#"{"message":"boom"}"#.to_string(),
            }]
        });
        let upd = gl_update(&base, "0.1.0");
        let res = upd.get_latest_releases();
        assert!(
            res.is_err(),
            "a non-2xx listing response must propagate as an error out of \
             get_latest_releases, got {:?}",
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
            matches!(res, Err(crate::errors::Error::Release(_))),
            "empty releases array must surface as Error::Release, got {:?}",
            res
        );
    }
}

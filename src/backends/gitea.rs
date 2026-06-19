/*!
gitea releases
*/
use crate::backends::common::{CommonBuilderConfig, CommonConfig, RequestConfig};
use crate::backends::{collect_paginated, first_page_url, next_link, send};
use crate::http_client::{header, HttpResponse};
use crate::version::bump_is_greater;
use crate::{
    errors::*,
    update::{Release, ReleaseAsset, ReleaseUpdate},
};

impl ReleaseAsset {
    /// Parse a release-asset json object
    ///
    /// Errors:
    ///     * Missing required name & download-url keys
    fn from_asset_gitea(asset: &serde_json::Value) -> Result<ReleaseAsset> {
        let download_url = asset["browser_download_url"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Asset missing `browser_download_url`"))?;
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
    fn from_release_gitea(release: &serde_json::Value) -> Result<Release> {
        let tag = release["tag_name"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `tag_name`"))?;
        let date = release["created_at"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `created_at`"))?;
        let name = release["name"].as_str().unwrap_or(tag);
        let assets = release["assets"]
            .as_array()
            .ok_or_else(|| format_err!(Error::Release, "No assets found"))?;
        let body = release["body"].as_str().map(String::from);
        let assets = assets
            .iter()
            .map(ReleaseAsset::from_asset_gitea)
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
    #[doc(alias = "url")]
    #[doc(alias = "with_host")]
    pub fn instance_url(&mut self, host: &str) -> &mut Self {
        self.host = Some(host.to_owned());
        self
    }

    /// Required. Set the repo owner, used to build a gitea api url
    pub fn repo_owner(&mut self, owner: &str) -> &mut Self {
        self.repo_owner = Some(owner.to_owned());
        self
    }

    /// Required. Set the repo name, used to build a gitea api url
    pub fn repo_name(&mut self, name: &str) -> &mut Self {
        self.repo_name = Some(name.to_owned());
        self
    }

    /// Set the optional arch `target` name, used to filter available releases
    #[doc(alias = "target")]
    #[doc(alias = "with_target")]
    pub fn filter_target(&mut self, target: &str) -> &mut Self {
        self.target = Some(target.to_owned());
        self
    }

    /// Set the authorization token, used in requests to the gitea api url
    ///
    /// This is to support private repos where you need a gitea auth token.
    /// **Make sure not to bake the token into your app**; it is recommended
    /// you obtain it via another mechanism, such as environment variables
    /// or prompting the user for input
    pub fn auth_token(&mut self, auth_token: &str) -> &mut Self {
        self.auth_token = Some(auth_token.to_owned());
        self
    }

    request_config_setters!(request);

    /// Verify builder args, returning a `ReleaseList`
    pub fn build(&self) -> Result<ReleaseList> {
        self.request.check()?;
        Ok(ReleaseList {
            host: if let Some(ref host) = self.host {
                host.to_owned()
            } else {
                bail!(
                    Error::Config,
                    "`instance_url` required (gitea has no default host; call `.instance_url(...)`)"
                )
            },
            repo_owner: if let Some(ref owner) = self.repo_owner {
                owner.to_owned()
            } else {
                bail!(Error::Config, "`repo_owner` required")
            },
            repo_name: if let Some(ref name) = self.repo_name {
                name.to_owned()
            } else {
                bail!(Error::Config, "`repo_name` required")
            },
            target: self.target.clone(),
            auth_token: self.auth_token.clone(),
            request: self.request.clone(),
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
    auth_token: Option<String>,
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

    /// Retrieve a list of `Release`s.
    /// If specified, filter for those containing a specified `target`
    pub fn fetch(&self) -> Result<Vec<Release>> {
        let api_url = format!(
            "{}/api/v1/repos/{}/{}/releases",
            self.host, self.repo_owner, self.repo_name
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
    #[doc(alias = "url")]
    #[doc(alias = "with_host")]
    pub fn instance_url(&mut self, host: &str) -> &mut Self {
        self.host = Some(host.to_owned());
        self
    }

    /// Required. Set the repo owner, used to build a gitea api url
    pub fn repo_owner(&mut self, owner: &str) -> &mut Self {
        self.repo_owner = Some(owner.to_owned());
        self
    }

    /// Required. Set the repo name, used to build a gitea api url
    pub fn repo_name(&mut self, name: &str) -> &mut Self {
        self.repo_name = Some(name.to_owned());
        self
    }

    impl_common_builder_setters!();

    /// Confirm config and create a ready-to-use `Update`
    ///
    /// * Errors:
    ///     * Config - Invalid `Update` configuration
    fn build_update(&self) -> Result<Update> {
        Ok(Update {
            host: if let Some(ref host) = self.host {
                host.to_owned()
            } else {
                bail!(
                    Error::Config,
                    "`instance_url` required (gitea has no default host; call `.instance_url(...)`)"
                )
            },
            repo_owner: if let Some(ref owner) = self.repo_owner {
                owner.to_owned()
            } else {
                bail!(Error::Config, "`repo_owner` required")
            },
            repo_name: if let Some(ref name) = self.repo_name {
                name.to_owned()
            } else {
                bail!(Error::Config, "`repo_name` required")
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

/// Updates to a specified or latest release distributed via gitea
#[derive(Debug)]
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
            self.host, self.repo_owner, self.repo_name
        )
    }
}

impl crate::update::sealed::Sealed for Update {}

impl ReleaseUpdate for Update {
    fn get_latest_release(&self) -> Result<Release> {
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
        Release::from_release_gitea(&releases[0])
    }

    fn get_latest_releases(&self, current_version: &str) -> Result<Vec<Release>> {
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

    fn get_release_version(&self, ver: &str) -> Result<Release> {
        let api_url = format!("{}/tags/{}", self.releases_url(), ver);
        let resp = send(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )?;
        let json = resp.json::<serde_json::Value>()?;
        Release::from_release_gitea(&json)
    }
}

impl_update_config_accessors!(Update, {
    fn api_headers(&self, auth_token: Option<&str>) -> Result<header::HeaderMap> {
        api_headers(auth_token)
    }
});

/// Fetch every release from `base_url`, following Gitea's `Link: rel="next"` pagination.
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
            .map(Release::from_release_gitea)
            .collect::<Result<Vec<Release>>>()?;
        Ok((releases, next_link(&headers)))
    })
}

/// Async sibling of [`fetch_all_releases`], following Gitea's `Link: rel="next"` pagination with
/// the async transport. Reuses the same [`Release::from_release_gitea`] parser.
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
                .map(Release::from_release_gitea)
                .collect::<Result<Vec<Release>>>()?;
            Ok((releases, next_link(&headers)))
        }
    })
    .await
}

#[cfg(feature = "async")]
impl crate::update::AsyncFetch for Update {
    async fn get_latest_release_async(&self) -> Result<Release> {
        use crate::backends::send_async;
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
        Release::from_release_gitea(&releases[0])
    }

    async fn get_latest_releases_async(&self, current_version: &str) -> Result<Vec<Release>> {
        let api_url = self.releases_url();
        let releases = fetch_all_releases_async(
            &api_url,
            self.common.auth_token.as_deref(),
            &self.common.request,
        )
        .await?;
        Ok(releases
            .into_iter()
            .filter(|r| bump_is_greater(current_version, &r.version).unwrap_or(false))
            .collect())
    }

    async fn get_release_version_async(&self, ver: &str) -> Result<Release> {
        use crate::backends::send_async;
        let api_url = format!("{}/tags/{}", self.releases_url(), ver);
        let resp = send_async(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )
        .await?;
        let json = resp.json::<serde_json::Value>().await?;
        Release::from_release_gitea(&json)
    }
}

fn api_headers(auth_token: Option<&str>) -> Result<header::HeaderMap> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        "rust-reqwest/self-update"
            .parse()
            .expect("gitea invalid user-agent"),
    );

    if let Some(token) = auth_token {
        headers.insert(
            header::AUTHORIZATION,
            format!("token {}", token)
                .parse()
                .map_err(|err| Error::Config(format!("Failed to parse auth token: {}", err)))?,
        );
    };

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::Update;

    #[cfg(feature = "async")]
    use std::io::{Read, Write};
    #[cfg(feature = "async")]
    use std::net::TcpListener;

    #[cfg(feature = "async")]
    struct Resp {
        status: &'static str,
        link: Option<String>,
        body: String,
    }

    /// Bind a loopback listener and serve `make(base_url)`'s responses in order, one per
    /// incoming connection, on a background thread. Returns the server's base URL
    /// (`http://127.0.0.1:<port>`). No external network is used.
    #[cfg(feature = "async")]
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
    #[cfg(feature = "async")]
    fn release_json(tag: &str) -> String {
        format!(
            r#"[{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[],"body":null}}]"#
        )
    }

    /// A JSON array of several releases (one object per `tag`), used by the
    /// `get_latest_releases_async` filtering test.
    #[cfg(feature = "async")]
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
    #[cfg(feature = "async")]
    fn release_obj_json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[],"body":null}}"#
        )
    }

    #[cfg(feature = "async")]
    fn gitea_update(base: &str, current_version: &str) -> Update {
        Update::configure()
            .instance_url(base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version(current_version)
            .build_async()
            .unwrap()
    }

    #[test]
    fn instance_url_and_filter_target_setters_exist_on_release_list_builder() {
        // The renamed `instance_url` / `filter_target` setters must exist on the gitea
        // `ReleaseListBuilder` and the builder must still build (gitea requires `instance_url`).
        let _list = super::ReleaseList::configure()
            .instance_url("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .filter_target("x86_64-unknown-linux-gnu")
            .build()
            .unwrap();
    }

    #[test]
    fn release_list_build_requires_instance_url() {
        // gitea has no default host, so the `ReleaseList` builder must error without `instance_url`.
        let res = super::ReleaseList::configure()
            .repo_owner("o")
            .repo_name("r")
            .build();
        assert!(matches!(res, Err(crate::errors::Error::Config(_))));
    }

    #[test]
    fn api_headers_override_uses_gitea_user_agent_and_token_scheme() {
        // The `{api_headers}` override arm must wire gitea's custom `api_headers` (User-Agent +
        // `token` auth scheme), not the trait default (which sets no User-Agent).
        let upd = Update::configure()
            .instance_url("https://gitea.example.com")
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
            "token secret",
            "gitea authenticates with the token scheme"
        );
    }

    #[test]
    fn release_list_build_surfaces_invalid_header() {
        // A bad header on the gitea `ReleaseListBuilder` must fail at `build()` via
        // `request.check()` with `Error::Config`, not panic. (The header check runs before the
        // host check, so a valid host is supplied to isolate the header failure.)
        let res = super::ReleaseList::configure()
            .instance_url("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .request_header("inva lid", "ok")
            .build();
        assert!(
            matches!(res, Err(crate::errors::Error::Config(_))),
            "invalid header must surface as Error::Config from gitea ReleaseList build()"
        );
    }

    #[test]
    fn update_build_surfaces_invalid_header() {
        // Same deferred-header check via `CommonBuilderConfig::build` on the gitea UpdateBuilder.
        let res = Update::configure()
            .instance_url("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .request_header("inva lid", "ok")
            .build();
        assert!(matches!(res, Err(crate::errors::Error::Config(_))));
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
            .instance_url("https://gitea.example.com")
            .repo_name("repo")
            .current_version("0.1.0")
            .build();
        assert!(missing_owner.is_err(), "build must fail without repo_owner");

        let missing_name = Update::configure()
            .instance_url("https://gitea.example.com")
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
            .instance_url("https://gitea.example.com")
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

    #[test]
    fn identifier_is_wired() {
        let upd = Update::configure()
            .instance_url("https://gitea.example.com")
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
            .instance_url("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(upd.bin_path_in_archive(), expected);

        // An explicit `bin_path_in_archive` set before `bin_name` is NOT overwritten.
        let upd = Update::configure()
            .instance_url("https://gitea.example.com")
            .repo_owner("o")
            .repo_name("r")
            .bin_path_in_archive("custom/path")
            .bin_name("app")
            .current_version("0.1.0")
            .build()
            .unwrap();
        assert_eq!(upd.bin_path_in_archive(), "custom/path");
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
            .instance_url(&base)
            .repo_owner("o")
            .repo_name("r")
            .bin_name("app")
            .current_version("0.1.0")
            .build_async()
            .unwrap();
        let rel = upd.get_latest_release_async().await.unwrap();
        assert_eq!(rel.version, "2.5.0");
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
        let releases = super::fetch_all_releases_async(
            &format!("{base}/api/v1/repos/o/r/releases"),
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
        use crate::update::AsyncFetch;
        let upd = gitea_update(&base, "0.1.0");
        let rel = upd.get_release_version_async("v4.2.1").await.unwrap();
        assert_eq!(rel.version, "4.2.1");
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
        use crate::update::AsyncFetch;
        let upd = gitea_update(&base, "0.1.0");
        let res = upd.get_release_version_async("v1.0.0").await;
        assert!(
            matches!(res, Err(crate::errors::Error::Release(_))),
            "missing tag_name must surface as Error::Release, got {:?}",
            res
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_releases_async_filters_to_newer_only() {
        // The single-page payload mixes releases newer than, equal to, and older than the current
        // version. `get_latest_releases_async` must keep only the strictly-newer ones, preserving
        // source order.
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v2.0.0", "v1.5.0", "v1.0.0", "v0.9.0"]),
            }]
        });
        use crate::update::AsyncFetch;
        let upd = gitea_update(&base, "1.0.0");
        let releases = upd.get_latest_releases_async("1.0.0").await.unwrap();
        let versions: Vec<&str> = releases.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(
            versions,
            vec!["2.0.0", "1.5.0"],
            "only releases strictly newer than the current version are kept, in order"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_releases_async_empty_when_all_older_or_equal() {
        // When no release is newer than the current version, the filtered result is empty
        // (this is the "up to date" signal the higher-level async update flow relies on).
        let base = stub(|_| {
            vec![Resp {
                status: "200 OK",
                link: None,
                body: releases_json(&["v1.0.0", "v0.9.0"]),
            }]
        });
        use crate::update::AsyncFetch;
        let upd = gitea_update(&base, "1.0.0");
        let releases = upd.get_latest_releases_async("1.0.0").await.unwrap();
        assert!(
            releases.is_empty(),
            "no release newer than current => empty list, got {:?}",
            releases
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_releases_async_accumulates_across_pages_then_filters() {
        // Filtering must happen *after* pagination: a newer release living on page 2 (reached via
        // the `Link: rel="next"` header) must still be retained.
        let base = stub(|base| {
            vec![
                Resp {
                    status: "200 OK",
                    link: Some(format!("{base}/api/v1/repos/o/r/releases?page=2")),
                    body: releases_json(&["v0.5.0"]),
                },
                Resp {
                    status: "200 OK",
                    link: None,
                    body: releases_json(&["v3.0.0"]),
                },
            ]
        });
        use crate::update::AsyncFetch;
        let upd = gitea_update(&base, "1.0.0");
        let releases = upd.get_latest_releases_async("1.0.0").await.unwrap();
        let versions: Vec<&str> = releases.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(
            versions,
            vec!["3.0.0"],
            "the newer release on page 2 survives pagination + filtering"
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
            matches!(res, Err(crate::errors::Error::Release(_))),
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
            matches!(res, Err(crate::errors::Error::Release(_))),
            "non-array payload must surface as Error::Release, got {:?}",
            res
        );
    }
}

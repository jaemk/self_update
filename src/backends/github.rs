/*!
GitHub releases
*/
use crate::http_client::{header, HeaderMap, HttpResponse};

use crate::backends::common::{CommonBuilderConfig, CommonConfig, RequestConfig};
use crate::backends::{collect_paginated, first_page_url, next_link, send};
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
    fn from_asset(asset: &serde_json::Value) -> Result<ReleaseAsset> {
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
    fn from_release(release: &serde_json::Value) -> Result<Release> {
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
            .map(ReleaseAsset::from_asset)
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
    repo_owner: Option<String>,
    repo_name: Option<String>,
    target: Option<String>,
    auth_token: Option<String>,
    custom_url: Option<String>,
    request: RequestConfig,
}
impl ReleaseListBuilder {
    /// Required. Set the repo owner, used to build a github api url
    pub fn repo_owner(&mut self, owner: &str) -> &mut Self {
        self.repo_owner = Some(owner.to_owned());
        self
    }

    /// Required. Set the repo name, used to build a github api url
    pub fn repo_name(&mut self, name: &str) -> &mut Self {
        self.repo_name = Some(name.to_owned());
        self
    }

    /// Set the optional arch `target` name, used to filter available releases
    #[doc(alias = "target")]
    pub fn filter_target(&mut self, target: &str) -> &mut Self {
        self.target = Some(target.to_owned());
        self
    }

    /// Set the optional github url, e.g. for a github enterprise installation.
    /// The url should provide the path to your API endpoint and end without a trailing slash,
    /// for example `https://api.github.com` or `https://github.mycorp.com/api/v3`
    pub fn url(&mut self, url: &str) -> &mut Self {
        self.custom_url = Some(url.to_owned());
        self
    }

    /// Set the authorization token, used in requests to the github api url
    ///
    /// This is to support private repos where you need a GitHub auth token.
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
            custom_url: self.custom_url.clone(),
            request: self.request.clone(),
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
    auth_token: Option<String>,
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

    /// Retrieve a list of `Release`s.
    /// If specified, filter for those containing a specified `target`
    pub fn fetch(&self) -> Result<Vec<Release>> {
        let api_url = format!(
            "{}/repos/{}/{}/releases",
            self.custom_url
                .as_ref()
                .unwrap_or(&"https://api.github.com".to_string()),
            self.repo_owner,
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
    pub fn repo_owner(&mut self, owner: &str) -> &mut Self {
        self.repo_owner = Some(owner.to_owned());
        self
    }

    /// Required. Set the repo name, used to build a github api url
    pub fn repo_name(&mut self, name: &str) -> &mut Self {
        self.repo_name = Some(name.to_owned());
        self
    }

    /// Set the optional github url, e.g. for a github enterprise installation.
    /// The url should provide the path to your API endpoint and end without a trailing slash,
    /// for example `https://api.github.com` or `https://github.mycorp.com/api/v3`
    pub fn url(&mut self, url: &str) -> &mut Self {
        self.custom_url = Some(url.to_owned());
        self
    }

    impl_common_builder_setters!();

    fn build_update(&self) -> Result<Update> {
        Ok(Update {
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
            custom_url: self.custom_url.clone(),
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

/// Updates to a specified or latest release distributed via GitHub
#[derive(Debug)]
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

impl ReleaseUpdate for Update {
    fn get_latest_release(&self) -> Result<Release> {
        let api_url = format!(
            "{}/repos/{}/{}/releases/latest",
            self.api_base(),
            self.repo_owner,
            self.repo_name
        );
        let resp = send(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )?;
        if !resp.status().is_success() {
            bail!(
                Error::Network,
                "api request failed with status: {:?} - for: {:?}",
                resp.status(),
                api_url
            )
        }
        let json = resp.json::<serde_json::Value>()?;
        Release::from_release(&json)
    }

    fn get_latest_releases(&self, current_version: &str) -> Result<Vec<Release>> {
        let api_url = format!(
            "{}/repos/{}/{}/releases",
            self.api_base(),
            self.repo_owner,
            self.repo_name
        );
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
        let api_url = format!(
            "{}/repos/{}/{}/releases/tags/{}",
            self.api_base(),
            self.repo_owner,
            self.repo_name,
            ver
        );
        let resp = send(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )?;
        if !resp.status().is_success() {
            bail!(
                Error::Network,
                "api request failed with status: {:?} - for: {:?}",
                resp.status(),
                api_url
            )
        }
        let json = resp.json::<serde_json::Value>()?;
        Release::from_release(&json)
    }
}

impl_update_config_accessors!(Update, {
    fn api_headers(&self, auth_token: Option<&str>) -> Result<HeaderMap> {
        api_headers(auth_token)
    }
});

/// Fetch every release from `base_url`, following GitHub's `Link: rel="next"` pagination.
fn fetch_all_releases(
    base_url: &str,
    auth_token: Option<&str>,
    req: &RequestConfig,
) -> Result<Vec<Release>> {
    collect_paginated(&first_page_url(base_url), |url| {
        let resp = send(url, api_headers(auth_token)?, req)?;
        if !resp.status().is_success() {
            bail!(
                Error::Network,
                "api request failed with status: {:?} - for: {:?}",
                resp.status(),
                url
            )
        }
        let headers = resp.headers().clone();
        let releases = resp
            .json::<serde_json::Value>()?
            .as_array()
            .ok_or_else(|| format_err!(Error::Release, "No releases found"))?
            .iter()
            .map(Release::from_release)
            .collect::<Result<Vec<Release>>>()?;
        Ok((releases, next_link(&headers)))
    })
}

/// Async sibling of [`fetch_all_releases`], following `Link: rel="next"` pagination with the async
/// transport. Reuses the same [`Release::from_release`] parser.
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
                .map(Release::from_release)
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
        let api_url = format!(
            "{}/repos/{}/{}/releases/latest",
            self.api_base(),
            self.repo_owner,
            self.repo_name
        );
        let resp = send_async(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )
        .await?;
        let json = resp.json::<serde_json::Value>().await?;
        Release::from_release(&json)
    }

    async fn get_latest_releases_async(&self, current_version: &str) -> Result<Vec<Release>> {
        let api_url = format!(
            "{}/repos/{}/{}/releases",
            self.api_base(),
            self.repo_owner,
            self.repo_name
        );
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
        let api_url = format!(
            "{}/repos/{}/{}/releases/tags/{}",
            self.api_base(),
            self.repo_owner,
            self.repo_name,
            ver
        );
        let resp = send_async(
            &api_url,
            api_headers(self.common.auth_token.as_deref())?,
            &self.common.request,
        )
        .await?;
        let json = resp.json::<serde_json::Value>().await?;
        Release::from_release(&json)
    }
}

fn api_headers(auth_token: Option<&str>) -> Result<header::HeaderMap> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        "rust/self-update"
            .parse()
            .expect("github invalid user-agent"),
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

    fn release_json(tag: &str) -> String {
        format!(
            r#"[{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[]}}]"#
        )
    }

    #[cfg(feature = "async")]
    fn release_obj_json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","created_at":"2020-01-01T00:00:00Z","name":"{tag}","assets":[]}}"#
        )
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
        let releases = super::fetch_all_releases_async(
            &format!("{base}/releases"),
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
            .url(&base)
            .build_async()
            .unwrap();
        let rel = upd.get_latest_release_async().await.unwrap();
        assert_eq!(rel.version, "3.1.0");
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
            .url(&base)
            .no_confirm(true)
            .show_output(false)
            .build_async()
            .unwrap();
        let status = upd.update_extended_async().await.unwrap();
        assert!(status.uptodate(), "an older release means up-to-date");
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
        let releases = super::fetch_all_releases(
            &format!("{base}/releases"),
            None,
            &crate::backends::common::RequestConfig::default(),
        )
        .unwrap();
        assert_eq!(
            releases.len(),
            2,
            "releases from both pages are accumulated"
        );
        assert_eq!(releases[0].version, "1.0.0");
        assert_eq!(releases[1].version, "0.9.0");
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
        let res = super::fetch_all_releases(
            &format!("{base}/releases"),
            None,
            &crate::backends::common::RequestConfig::default(),
        );
        // A non-2xx status is always an error, though the variant differs by client:
        // `reqwest` returns the response (mapped to `Network` by our status check) while
        // `ureq` fails the request itself (mapped to `Http`).
        assert!(matches!(
            res,
            Err(crate::errors::Error::Network(_)) | Err(crate::errors::Error::Http(_))
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
        let res = super::fetch_all_releases(
            &format!("{base}/releases"),
            None,
            &crate::backends::common::RequestConfig::default(),
        );
        assert!(matches!(res, Err(crate::errors::Error::Release(_))));
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
    fn api_headers_override_uses_github_user_agent_and_token_scheme() {
        // The `{api_headers}` override arm of `impl_update_config_accessors!` must wire github's
        // custom `api_headers` (its `rust/self-update` User-Agent + `token` auth scheme), not the
        // trait default (which sets no User-Agent).
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
            "rust/self-update"
        );
        assert_eq!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "token secret",
            "github authenticates with the token scheme"
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
            matches!(res, Err(crate::errors::Error::Config(_))),
            "invalid header name should surface as Error::Config from build()"
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
            .verify_with(|_new_exe| true)
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
            .checksum(crate::Checksum::Sha256("ab".repeat(32)))
            .build()
            .unwrap();
        assert!(upd.checksum().is_some());
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
        let release = Release {
            assets: vec![
                ReleaseAsset {
                    name: "app-stable.bin".into(),
                    download_url: "https://example/stable".into(),
                },
                ReleaseAsset {
                    name: "app-nightly.bin".into(),
                    download_url: "https://example/nightly".into(),
                },
            ],
            ..Default::default()
        };

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
        let chosen = matcher(&release.assets).expect("matcher selects an asset");
        assert_eq!(chosen.name, "app-nightly.bin");
        assert_eq!(chosen.download_url, "https://example/nightly");
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
        assert!(upd.request_client().blocking.is_some());
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
        let mut cfg = RequestConfig::default();
        cfg.client.blocking = Some(client);
        let releases = super::fetch_all_releases(&format!("{base}/releases"), None, &cfg).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "1.2.3");
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
        assert!(upd.request_client().r#async.is_some());
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
        let mut cfg = RequestConfig::default();
        cfg.client.r#async = Some(client);
        let releases = super::fetch_all_releases_async(&format!("{base}/releases"), None, &cfg)
            .await
            .unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "2.0.0");
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
        assert!(upd.request_client().agent.is_some());

        // And the injected agent actually performs the request.
        let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
        let mut cfg = RequestConfig::default();
        cfg.client.agent = Some(agent);
        let releases = super::fetch_all_releases(&format!("{base}/releases"), None, &cfg).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "3.0.0");
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
        let releases = super::fetch_all_releases(&format!("{base}/releases"), None, &cfg).unwrap();
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
        let res = super::fetch_all_releases(&format!("{base}/releases"), None, &cfg);
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
        let releases = super::fetch_all_releases(&format!("{base}/releases"), None, &cfg).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "1.0.0");
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
        let res = super::fetch_all_releases(&format!("{base}/releases"), None, &cfg);
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
            .url(&base)
            .repo_owner("o")
            .repo_name("r")
            .retries(1)
            .build()
            .unwrap()
            .fetch()
            .unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "2.0.0");
    }
}

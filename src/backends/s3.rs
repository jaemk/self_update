/*!
Amazon S3 releases
*/
use crate::backends::common::{CommonBuilderConfig, CommonConfig, RequestConfig};
use crate::backends::send;
use crate::http_client::HttpResponse;
use crate::{
    errors::*,
    update::{Release, ReleaseAsset, ReleaseUpdate},
    version::bump_is_greater,
};
use log::debug;
use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;
use std::cmp::Ordering;
use std::path::PathBuf;

/// Maximum number of items to retrieve from S3 API
const MAX_KEYS: u8 = 100;

/// The service end point.
///
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub enum EndPoint {
    /// Short for `https://<bucket>.s3.<region>.amazonaws.com/`
    #[default]
    S3,
    /// Short for `https://<bucket>.s3.dualstack.<region>.amazonaws.com/`
    S3DualStack,
    /// Short for `https://storage.googleapis.com/<bucket>/`
    GCS,
    /// Short for `https://<bucket>.<region>.digitaloceanspaces.com/`
    DigitalOceanSpaces,
    /// Generic, for other s3 compatible providers
    Generic {
        /// The full URL of the end point. For example:
        ///
        /// - `https://bucket.s3.example.com/`
        /// - `https://s3.example.com/bucket/`
        end_point: String,
    },
}

impl From<&str> for EndPoint {
    fn from(value: &str) -> Self {
        Self::Generic {
            end_point: value.to_owned(),
        }
    }
}

impl From<String> for EndPoint {
    fn from(value: String) -> Self {
        Self::Generic { end_point: value }
    }
}

/// `ReleaseList` Builder
#[derive(Clone, Debug)]
#[must_use]
pub struct ReleaseListBuilder {
    end_point: EndPoint,
    bucket_name: Option<String>,
    asset_prefix: Option<String>,
    target: Option<String>,
    region: Option<String>,
    #[cfg(feature = "s3-auth")]
    access_key: Option<auth::AccessKey>,
    request: RequestConfig,
}

impl ReleaseListBuilder {
    /// Set the bucket name, used to build an S3 api url
    pub fn bucket_name(&mut self, name: &str) -> &mut Self {
        self.bucket_name = Some(name.to_owned());
        self
    }

    /// Set the optional asset name prefix, used to filter available assets with a prefix string
    pub fn asset_prefix(&mut self, prefix: &str) -> &mut Self {
        self.asset_prefix = Some(prefix.to_owned());
        self
    }

    /// Set the S3 region used in the download url
    pub fn region(&mut self, region: &str) -> &mut Self {
        self.region = Some(region.to_owned());
        self
    }

    /// Set the end point
    pub fn end_point(&mut self, end_point: impl Into<EndPoint>) -> &mut Self {
        self.end_point = end_point.into();
        self
    }

    /// Set the optional arch `target` name, used to filter available releases
    pub fn target(&mut self, target: &str) -> &mut Self {
        self.target = Some(target.to_owned());
        self
    }

    #[cfg(feature = "s3-auth")]
    /// Set the access key
    pub fn access_key(&mut self, access_key: impl Into<auth::AccessKey>) -> &mut Self {
        self.access_key = Some(access_key.into());
        self
    }

    request_config_setters!(request);

    /// Verify builder args, returning a `ReleaseList`
    pub fn build(&self) -> Result<ReleaseList> {
        Ok(ReleaseList {
            end_point: self.end_point.clone(),
            bucket_name: if let Some(ref name) = self.bucket_name {
                name.to_owned()
            } else {
                bail!(Error::Config, "`bucket_name` required")
            },
            region: self.region.clone(),
            asset_prefix: self.asset_prefix.clone(),
            target: self.target.clone(),
            #[cfg(feature = "s3-auth")]
            access_key: self.access_key.clone(),
            request: self.request.clone(),
        })
    }
}

/// `ReleaseList` provides a builder api for querying an S3 bucket,
/// returning a `Vec` of available `Release`s
#[derive(Clone, Debug)]
pub struct ReleaseList {
    end_point: EndPoint,
    bucket_name: String,
    asset_prefix: Option<String>,
    target: Option<String>,
    region: Option<String>,
    #[cfg(feature = "s3-auth")]
    access_key: Option<auth::AccessKey>,
    request: RequestConfig,
}

impl ReleaseList {
    /// Initialize a ReleaseListBuilder
    pub fn configure() -> ReleaseListBuilder {
        ReleaseListBuilder {
            end_point: EndPoint::default(),
            bucket_name: None,
            asset_prefix: None,
            target: None,
            region: None,
            #[cfg(feature = "s3-auth")]
            access_key: None,
            request: RequestConfig::default(),
        }
    }

    /// Retrieve a list of `Release`s.
    /// If specified, filter for those containing a specified `target`
    pub fn fetch(&self) -> Result<Vec<Release>> {
        let releases = fetch_releases_from_s3(
            &self.end_point,
            &self.bucket_name,
            &self.region,
            &self.asset_prefix,
            #[cfg(feature = "s3-auth")]
            &self.access_key,
            &self.request,
        )?;
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

/// `s3::Update` builder
///
/// Configure download and installation from
/// `https://<bucket_name>.s3.<region>.amazonaws.com/<asset filename>`
#[derive(Debug, Default)]
#[must_use]
pub struct UpdateBuilder {
    end_point: EndPoint,
    bucket_name: Option<String>,
    asset_prefix: Option<String>,
    region: Option<String>,
    #[cfg(feature = "s3-auth")]
    access_key: Option<auth::AccessKey>,
    common: CommonBuilderConfig,
}

/// Configure download and installation from bucket
impl UpdateBuilder {
    /// Initialize a new builder
    pub fn new() -> Self {
        Default::default()
    }

    /// Set the end point
    pub fn end_point(&mut self, end_point: impl Into<EndPoint>) -> &mut Self {
        self.end_point = end_point.into();
        self
    }

    /// Set the bucket name, used to build a s3 api url
    pub fn bucket_name(&mut self, name: &str) -> &mut Self {
        self.bucket_name = Some(name.to_owned());
        self
    }

    /// Set the optional asset name prefix, used to filter available assets with a prefix string
    pub fn asset_prefix(&mut self, prefix: &str) -> &mut Self {
        self.asset_prefix = Some(prefix.to_owned());
        self
    }

    /// Set the S3 region used in the download url
    pub fn region(&mut self, region: &str) -> &mut Self {
        self.region = Some(region.to_owned());
        self
    }

    #[cfg(feature = "s3-auth")]
    /// Set the access key (an `(access_key_id, secret_access_key)` pair)
    pub fn access_key(&mut self, access_key: impl Into<auth::AccessKey>) -> &mut Self {
        self.access_key = Some(access_key.into());
        self
    }

    impl_common_builder_setters!(no_auth_token);

    /// **Deprecated and a no-op on the S3 backend.** S3 authenticates by signing requests with
    /// an `access_key` (AWS SigV4), not a bearer token, so this setter has never had any effect
    /// here. Use [`access_key`](Self::access_key) instead. Retained for one release to avoid a
    /// hard break; it will be removed in the next major version.
    #[deprecated(
        since = "1.0.0",
        note = "S3 uses `access_key` (AWS SigV4 signing), not an auth token; `auth_token` is a \
                no-op on the S3 backend. Use `.access_key((id, secret))` instead."
    )]
    pub fn auth_token(&mut self, _auth_token: &str) -> &mut Self {
        self
    }

    fn build_update(&self) -> Result<Update> {
        Ok(Update {
            end_point: self.end_point.clone(),
            bucket_name: if let Some(ref name) = self.bucket_name {
                name.to_owned()
            } else {
                bail!(Error::Config, "`bucket_name` required")
            },
            region: self.region.clone(),
            #[cfg(feature = "s3-auth")]
            access_key: self.access_key.clone(),
            asset_prefix: self.asset_prefix.clone(),
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

/// Updates to a specified or latest release distributed via S3
#[derive(Debug)]
pub struct Update {
    end_point: EndPoint,
    bucket_name: String,
    asset_prefix: Option<String>,
    region: Option<String>,
    #[cfg(feature = "s3-auth")]
    access_key: Option<auth::AccessKey>,
    common: CommonConfig,
}

impl Update {
    /// Initialize a new `Update` builder
    pub fn configure() -> UpdateBuilder {
        UpdateBuilder::new()
    }

    /// Fetch the bucket's releases (sync). Wraps the per-backend argument plumbing so the
    /// `ReleaseUpdate` methods stay terse.
    fn fetch_releases(&self) -> Result<Vec<Release>> {
        fetch_releases_from_s3(
            &self.end_point,
            &self.bucket_name,
            &self.region,
            &self.asset_prefix,
            #[cfg(feature = "s3-auth")]
            &self.access_key,
            &self.common.request,
        )
    }

    /// Async sibling of [`fetch_releases`](Self::fetch_releases).
    #[cfg(feature = "async")]
    async fn fetch_releases_async(&self) -> Result<Vec<Release>> {
        fetch_releases_from_s3_async(
            &self.end_point,
            &self.bucket_name,
            &self.region,
            &self.asset_prefix,
            #[cfg(feature = "s3-auth")]
            &self.access_key,
            &self.common.request,
        )
        .await
    }
}

/// Pick the single highest-version release. Shared by the sync and async paths.
fn pick_latest(releases: &[Release]) -> Result<Release> {
    let rel = releases
        .iter()
        .max_by(|x, y| match bump_is_greater(&y.version, &x.version) {
            Ok(is_greater) => {
                if is_greater {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            // Ignoring release due to an unexpected failure in parsing its version string
            Err(_) => Ordering::Less,
        });
    match rel {
        Some(r) => Ok(r.clone()),
        None => bail!(Error::Release, "No release was found"),
    }
}

/// Filter releases newer than `current_version`, sorted newest-first (the orchestrator takes the
/// first compatible one). Shared by the sync and async paths.
fn sort_newer(releases: Vec<Release>, current_version: &str) -> Vec<Release> {
    let mut releases = releases
        .into_iter()
        .filter(|r| bump_is_greater(current_version, &r.version).unwrap_or(false))
        .collect::<Vec<_>>();
    // Descending order (latest first), since the update code takes `.first()`.
    releases.sort_by(|x, y| match bump_is_greater(&y.version, &x.version) {
        Ok(is_greater) => {
            if is_greater {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        // Ignoring release due to an unexpected failure in parsing its version string
        Err(_) => Ordering::Greater,
    });
    releases
}

/// Find the release matching an explicit version. Shared by the sync and async paths.
fn find_version(releases: &[Release], ver: &str) -> Result<Release> {
    match releases.iter().find(|x| x.version == ver) {
        Some(r) => Ok(r.clone()),
        None => bail!(
            Error::Release,
            "No release with version '{}' was found",
            ver
        ),
    }
}

impl crate::update::sealed::Sealed for Update {}

impl ReleaseUpdate for Update {
    fn get_latest_release(&self) -> Result<Release> {
        pick_latest(&self.fetch_releases()?)
    }

    fn get_latest_releases(&self, current_version: &str) -> Result<Vec<Release>> {
        Ok(sort_newer(self.fetch_releases()?, current_version))
    }

    fn get_release_version(&self, ver: &str) -> Result<Release> {
        find_version(&self.fetch_releases()?, ver)
    }

    impl_release_update_accessors!();
}

#[cfg(feature = "async")]
impl crate::update::AsyncReleaseSource for Update {
    async fn get_latest_release_async(&self) -> Result<Release> {
        pick_latest(&self.fetch_releases_async().await?)
    }

    async fn get_latest_releases_async(&self, current_version: &str) -> Result<Vec<Release>> {
        Ok(sort_newer(
            self.fetch_releases_async().await?,
            current_version,
        ))
    }

    async fn get_release_version_async(&self, ver: &str) -> Result<Release> {
        find_version(&self.fetch_releases_async().await?, ver)
    }
}

/// Generate S3 auth parameters
#[cfg(feature = "s3-auth")]
mod auth {
    use crate::errors::*;
    use hmac::{Hmac, Mac};
    use percent_encoding::{utf8_percent_encode, AsciiSet, PercentEncode, NON_ALPHANUMERIC};
    use sha2::{Digest, Sha256};
    use std::{
        borrow::Cow,
        time::{SystemTime, UNIX_EPOCH},
    };
    use time::OffsetDateTime;
    use url::Url;

    #[derive(Clone, Debug)]
    pub struct AccessKey {
        pub access_key_id: String,
        pub secret_access_key: String,
    }

    impl From<(&str, &str)> for AccessKey {
        fn from(value: (&str, &str)) -> Self {
            Self {
                access_key_id: value.0.to_owned(),
                secret_access_key: value.1.to_owned(),
            }
        }
    }

    impl From<(String, String)> for AccessKey {
        fn from(value: (String, String)) -> Self {
            Self {
                access_key_id: value.0,
                secret_access_key: value.1,
            }
        }
    }

    // NON_ALPHANUMERIC Encodes everything except A-Z, a-z, 0-9.
    // Remove the last 4 reserved characters that AWS doesn't encode: - . _ ~
    const URI_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');

    // AWS doesn't encode the slash character in the canonical URI, but it does
    // encode it in query parameters
    const URI_ENCODE_KEEP_SLASH: &AsciiSet = &URI_ENCODE.remove(b'/');

    // Encode a string for use in AWS S3 signature v4, encoding reserved
    // characters and optionally the slash character
    fn uri_encode(input: &str, encode_slash: bool) -> PercentEncode<'_> {
        let set = if encode_slash {
            URI_ENCODE
        } else {
            URI_ENCODE_KEEP_SLASH
        };
        utf8_percent_encode(input, set)
    }

    fn hex_sha256(data: &[u8]) -> String {
        let hash = Sha256::digest(data);
        hash.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
        let mut mac = Hmac::<Sha256>::new_from_slice(key)?;
        mac.update(data);
        Ok(mac.finalize().into_bytes().to_vec())
    }

    fn derive_signing_key(secret: &str, date_stamp: &str, region: &str) -> Result<Vec<u8>> {
        let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes())?;
        let k_region = hmac_sha256(&k_date, region.as_bytes())?;
        let k_service = hmac_sha256(&k_region, b"s3")?;
        hmac_sha256(&k_service, b"aws4_request")
    }

    fn format_timestamp(secs: u64) -> Result<(String, String)> {
        let dt = OffsetDateTime::from_unix_timestamp(secs as i64)?;
        let date_stamp = format!("{:04}{:02}{:02}", dt.year(), dt.month() as u8, dt.day());
        let amz_date = format!(
            "{date_stamp}T{:02}{:02}{:02}Z",
            dt.hour(),
            dt.minute(),
            dt.second()
        );
        Ok((date_stamp, amz_date))
    }

    pub fn s3_signature_v4(
        url_str: &str,
        region: &Option<String>,
        access_key: &Option<AccessKey>,
        ttl_secs: u64,
    ) -> Result<String> {
        let (access_key_id, secret_access_key) = match access_key {
            Some(access_key) => (&access_key.access_key_id, &access_key.secret_access_key),
            None => return Ok(url_str.to_owned()),
        };
        let url = Url::parse(url_str)?;
        let host = url
            .host_str()
            .ok_or_else(|| Error::Config(format!("Cannot extract host from {:?}", url_str)))?;
        let canonical_uri = if url.path().is_empty() {
            "/"
        } else {
            url.path()
        };

        let now_secs = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let (date_stamp, amz_date) = format_timestamp(now_secs)?;

        let region = region.as_deref().unwrap_or("us-east-1");

        let credential_scope = format!("{date_stamp}/{region}/s3/aws4_request");

        // Existing query params (decoded by url crate) + SigV4 params, sans Signature.
        let mut params: Vec<_> = url.query_pairs().collect();

        params.extend([
            (
                Cow::Borrowed("X-Amz-Algorithm"),
                Cow::Borrowed("AWS4-HMAC-SHA256"),
            ),
            (
                Cow::Borrowed("X-Amz-Credential"),
                Cow::Owned(format!("{access_key_id}/{credential_scope}")),
            ),
            (Cow::Borrowed("X-Amz-Date"), Cow::Borrowed(&amz_date)),
            (
                Cow::Borrowed("X-Amz-Expires"),
                Cow::Owned(ttl_secs.to_string()),
            ),
            (Cow::Borrowed("X-Amz-SignedHeaders"), Cow::Borrowed("host")),
        ]);
        params.sort_by(|a, b| a.0.cmp(&b.0));

        let canonical_qs: String = params
            .iter()
            .map(|(k, v)| format!("{}={}", uri_encode(k, true), uri_encode(v, true)))
            .collect::<Vec<_>>()
            .join("&");

        let canonical_request = format!(
            "GET\n{}\n{canonical_qs}\nhost:{host}\n\nhost\nUNSIGNED-PAYLOAD",
            uri_encode(canonical_uri, false),
        );

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            hex_sha256(canonical_request.as_bytes())
        );

        let signing_key = derive_signing_key(secret_access_key, &date_stamp, region)?;
        let signature: String = hmac_sha256(&signing_key, string_to_sign.as_bytes())?
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();

        let base = &url_str[..url_str.find('?').unwrap_or(url_str.len())];
        Ok(format!("{base}?{canonical_qs}&X-Amz-Signature={signature}"))
    }
}

/// Build the S3 listing `api_url` and the `download_base_url` that asset URLs are formed against,
/// signing the listing URL when `s3-auth` is enabled. Shared by the sync and async fetch paths.
fn build_s3_api_url(
    end_point: &EndPoint,
    bucket_name: &str,
    region: &Option<String>,
    asset_prefix: &Option<String>,
    #[cfg(feature = "s3-auth")] access_key: &Option<auth::AccessKey>,
) -> Result<(String, String)> {
    let prefix = match asset_prefix {
        Some(prefix) => format!("&prefix={}", prefix),
        None => "".to_string(),
    };

    let region_result = region
        .as_ref()
        .ok_or_else(|| Error::Config("`region` required".to_string()));

    let download_base_url = match end_point {
        EndPoint::S3 => format!(
            "https://{}.s3.{}.amazonaws.com/",
            bucket_name, region_result?
        ),
        EndPoint::S3DualStack => format!(
            "https://{}.s3.dualstack.{}.amazonaws.com/",
            bucket_name, region_result?
        ),
        EndPoint::DigitalOceanSpaces => format!(
            "https://{}.{}.digitaloceanspaces.com/",
            bucket_name, region_result?
        ),
        EndPoint::GCS => format!("https://storage.googleapis.com/{}/", bucket_name),
        EndPoint::Generic { ref end_point } => end_point.clone(),
    };

    let api_url = match end_point {
        EndPoint::S3
        | EndPoint::S3DualStack
        | EndPoint::DigitalOceanSpaces
        | EndPoint::Generic { .. } => format!(
            "{}?list-type=2&max-keys={}{}",
            download_base_url, MAX_KEYS, prefix
        ),
        EndPoint::GCS => format!("{}?max-keys={}{}", download_base_url, MAX_KEYS, prefix),
    };

    #[cfg(feature = "s3-auth")]
    let api_url = auth::s3_signature_v4(&api_url, region, access_key, 300)?;

    Ok((download_base_url, api_url))
}

/// Obtain list of releases from AWS S3 API, from bucket and region specified,
/// filtering assets which don't match the prefix string if provided.
///
/// This will strip the prefix from provided file names, allowing use with subdirectories
fn fetch_releases_from_s3(
    end_point: &EndPoint,
    bucket_name: &str,
    region: &Option<String>,
    asset_prefix: &Option<String>,
    #[cfg(feature = "s3-auth")] access_key: &Option<auth::AccessKey>,
    req: &RequestConfig,
) -> Result<Vec<Release>> {
    let (download_base_url, api_url) = build_s3_api_url(
        end_point,
        bucket_name,
        region,
        asset_prefix,
        #[cfg(feature = "s3-auth")]
        access_key,
    )?;

    debug!("using api url: {:?}", api_url);

    // `send` already errored on a non-success status (see `fetch_releases_from_s3_async`).
    let resp = send(&api_url, Default::default(), req)?;
    let body = resp.text()?;
    parse_s3_response(
        &body,
        &download_base_url,
        #[cfg(feature = "s3-auth")]
        region,
        #[cfg(feature = "s3-auth")]
        access_key,
    )
}

/// Async sibling of [`fetch_releases_from_s3`], reusing [`build_s3_api_url`] and
/// [`parse_s3_response`] with the async transport.
#[cfg(feature = "async")]
async fn fetch_releases_from_s3_async(
    end_point: &EndPoint,
    bucket_name: &str,
    region: &Option<String>,
    asset_prefix: &Option<String>,
    #[cfg(feature = "s3-auth")] access_key: &Option<auth::AccessKey>,
    req: &RequestConfig,
) -> Result<Vec<Release>> {
    use crate::backends::send_async;
    let (download_base_url, api_url) = build_s3_api_url(
        end_point,
        bucket_name,
        region,
        asset_prefix,
        #[cfg(feature = "s3-auth")]
        access_key,
    )?;

    debug!("using api url: {:?}", api_url);

    let resp = send_async(&api_url, Default::default(), req).await?;
    let body = resp.text().await?;
    parse_s3_response(
        &body,
        &download_base_url,
        #[cfg(feature = "s3-auth")]
        region,
        #[cfg(feature = "s3-auth")]
        access_key,
    )
}

/// Parse an S3 `ListBucketResult` XML body into releases, forming (and, under `s3-auth`, signing)
/// each asset's download URL against `download_base_url`. Pure/sync — shared by both fetch paths.
fn parse_s3_response(
    body: &str,
    download_base_url: &str,
    #[cfg(feature = "s3-auth")] region: &Option<String>,
    #[cfg(feature = "s3-auth")] access_key: &Option<auth::AccessKey>,
) -> Result<Vec<Release>> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);

    // Let's now parse the response to extract the releases
    enum Tag {
        Contents,
        Key,
        LastModified,
        Other,
    }

    let mut current_tag = Tag::Other;
    let mut current_release: Option<Release> = None;
    let regex =
        Regex::new(r"(?i)(?P<prefix>.*/)*(?P<name>.+)-[v]{0,1}(?P<version>\d+\.\d+\.\d+)-.+")
            .map_err(|err| {
                Error::Release(format!(
                    "Failed constructing regex to parse S3 filenames: {}",
                    err
                ))
            })?;

    // inspecting each XML element we populate our releases list
    let mut buf = Vec::new();
    let mut releases: Vec<Release> = vec![];
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().into_inner() {
                b"Contents" => {
                    current_tag = Tag::Contents;
                    if let Some(release) = current_release {
                        add_to_releases_list(&mut releases, release);
                    }
                    current_release = None;
                }
                b"Key" => current_tag = Tag::Key,
                b"LastModified" => current_tag = Tag::LastModified,
                _ => current_tag = Tag::Other,
            },
            Ok(Event::Text(e)) => {
                // if we cannot decode a tag text we just ignore it
                if let Ok(txt) = e.decode().map(|r| r.into_owned()) {
                    match current_tag {
                        Tag::Key => {
                            let p = PathBuf::from(&txt);
                            let exe_name = match p.file_name().map(|v| v.to_str()) {
                                Some(Some(v)) => v,
                                _ => &txt,
                            };

                            if let Some(captures) = regex.captures(&txt) {
                                let release = current_release.get_or_insert(Release::default());
                                release.name = captures["name"].to_string();
                                release.version =
                                    captures["version"].trim_start_matches('v').to_string();
                                let download_url = format!("{}{}", download_base_url, txt);

                                #[cfg(feature = "s3-auth")]
                                let download_url =
                                    auth::s3_signature_v4(&download_url, region, access_key, 300)?;

                                release.assets = vec![ReleaseAsset {
                                    name: exe_name.to_string(),
                                    download_url,
                                }];
                                debug!("Matched release: {:?}", release);
                            } else {
                                debug!("Regex mismatch: {:?}", &txt);
                            }
                        }
                        Tag::LastModified => {
                            let release = current_release.get_or_insert(Release::default());
                            release.date = txt;
                        }
                        _ => (),
                    }
                }
            }
            Ok(Event::Eof) => {
                if let Some(release) = current_release {
                    add_to_releases_list(&mut releases, release);
                }
                break; // exits the loop when reaching end of file
            }
            Err(e) => bail!(
                Error::Release,
                "Failed when parsing S3 XML response at position {}: {:?}",
                reader.buffer_position(),
                e
            ),
            _ => (), // There are several other `Event`s we ignore here
        }

        buf.clear();
    }

    Ok(releases)
}

// Add a release to the list if it's doesn't exist yet, or merge its asset/s
// details into the release item already existing in the list
fn add_to_releases_list(releases: &mut Vec<Release>, mut rel: Release) {
    if !rel.version.is_empty() && !rel.name.is_empty() {
        match releases
            .iter()
            .position(|curr| curr.name == rel.name && curr.version == rel.version)
        {
            Some(index) => {
                rel.assets.append(&mut releases[index].assets);
                releases.push(rel);
                releases.swap_remove(index);
            }
            None => releases.push(rel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Update;
    use crate::update::{Release, ReleaseUpdate};

    fn rel(version: &str) -> Release {
        Release::builder().version(version).build().unwrap()
    }

    #[test]
    fn pick_latest_returns_highest_version() {
        let releases = [rel("1.0.0"), rel("2.3.1"), rel("2.0.0"), rel("1.9.9")];
        assert_eq!(super::pick_latest(&releases).unwrap().version, "2.3.1");
    }

    #[test]
    fn pick_latest_errors_on_empty() {
        assert!(super::pick_latest(&[]).is_err());
    }

    #[test]
    fn sort_newer_keeps_only_newer_descending() {
        let releases = vec![rel("0.9.0"), rel("1.5.0"), rel("1.0.0"), rel("2.0.0")];
        let newer = super::sort_newer(releases, "1.0.0");
        // 0.9.0 and 1.0.0 are not strictly newer than 1.0.0; the rest are, newest-first.
        let versions: Vec<_> = newer.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(versions, vec!["2.0.0", "1.5.0"]);
    }

    #[test]
    fn find_version_matches_exact() {
        let releases = [rel("1.0.0"), rel("1.2.3"), rel("2.0.0")];
        assert_eq!(
            super::find_version(&releases, "1.2.3").unwrap().version,
            "1.2.3"
        );
        assert!(super::find_version(&releases, "9.9.9").is_err());
    }

    fn configured() -> Box<dyn ReleaseUpdate> {
        Update::configure()
            .bucket_name("bucket")
            .asset_prefix("prefix")
            .region("us-east-1")
            .bin_name("my_bin")
            .target("x86_64-unknown-linux-gnu")
            .identifier("musl")
            .current_version("0.1.0")
            .build()
            .unwrap()
    }

    #[test]
    fn identifier_and_str_accessors_are_wired() {
        let upd = configured();
        // `identifier` is newly supported on the s3 builder.
        assert_eq!(upd.identifier(), Some("musl"));
        assert_eq!(upd.target(), "x86_64-unknown-linux-gnu");
        assert_eq!(upd.current_version(), "0.1.0");
    }

    #[test]
    fn s3_auth_token_is_a_deprecated_noop() {
        // The s3 backend overrides the shared `auth_token` setter with a `#[deprecated]` no-op
        // (s3 authenticates via `access_key`/SigV4). Calling it stores nothing.
        #[allow(deprecated)]
        let upd = Update::configure()
            .bucket_name("bucket")
            .region("us-east-1")
            .bin_name("my_bin")
            .current_version("0.1.0")
            .auth_token("ignored")
            .build()
            .unwrap();
        assert_eq!(upd.auth_token(), None);
    }

    #[test]
    fn default_api_headers_rejects_invalid_token_without_panicking() {
        let upd = configured();
        // A token containing a newline is not a valid HTTP header value. The default
        // `api_headers` impl must surface an error rather than panic (it previously
        // `unwrap()`ed the parse).
        assert!(upd.api_headers(Some("bad\ntoken")).is_err());
        // A well-formed token still succeeds.
        assert!(upd.api_headers(Some("good-token")).is_ok());
    }
}

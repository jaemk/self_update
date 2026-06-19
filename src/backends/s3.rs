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

/// Re-export the S3 [`AccessKey`] credential type at the backend module level so consumers can
/// name it as `self_update::backends::s3::AccessKey` (e.g. to build one explicitly). Available
/// under the `s3-auth` feature.
#[cfg(feature = "s3-auth")]
pub use auth::AccessKey;

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
    #[doc(alias = "target")]
    #[doc(alias = "with_target")]
    pub fn filter_target(&mut self, target: &str) -> &mut Self {
        self.target = Some(target.to_owned());
        self
    }

    #[cfg(feature = "s3-auth")]
    /// Set the access key
    #[doc(alias = "access_key_id")]
    pub fn access_key(&mut self, access_key: impl Into<auth::AccessKey>) -> &mut Self {
        self.access_key = Some(access_key.into());
        self
    }

    request_config_setters!(request);

    /// Verify builder args, returning a `ReleaseList`
    pub fn build(&self) -> Result<ReleaseList> {
        self.request.check()?;
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
#[derive(Clone, Debug, Default)]
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
    #[doc(alias = "access_key_id")]
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
}

impl_update_config_accessors!(Update);

#[cfg(feature = "async")]
impl crate::update::AsyncFetch for Update {
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

    /// S3 access credentials used to sign requests (AWS SigV4) for private buckets.
    ///
    /// Construct one from an `(access_key_id, secret_access_key)` pair via [`From`] (e.g.
    /// `("AKIA…", "secret").into()`), which is what [`access_key`](super::UpdateBuilder::access_key)
    /// accepts. It is `#[non_exhaustive]` so future credential fields (e.g. an STS session token)
    /// can be added without a breaking change; build it through the `From` impls rather than a
    /// struct literal.
    #[derive(Clone, Debug)]
    #[non_exhaustive]
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

    // `http_client::get` bails on any non-2xx status before returning the response.
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

    // ---------------------------------------------------------------------------
    // Helpers shared between sync XML-parse tests and async stub tests
    // ---------------------------------------------------------------------------

    /// Build a minimal `ListBucketResult` XML body with the given `<Key>` entries.
    fn list_bucket_xml(keys: &[&str]) -> String {
        let contents: String = keys
            .iter()
            .map(|k| {
                format!(
                    "<Contents><Key>{k}</Key><LastModified>2024-01-01T00:00:00.000Z</LastModified></Contents>"
                )
            })
            .collect();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
             <Name>my-bucket</Name>{contents}</ListBucketResult>"
        )
    }

    // ---------------------------------------------------------------------------
    // parse_s3_response / add_to_releases_list unit tests (no network)
    // ---------------------------------------------------------------------------

    #[test]
    fn parse_s3_response_single_release_single_asset() {
        // One <Contents> entry that matches the version regex: name="myapp", version="1.2.3",
        // suffix "-x86_64-linux". The trailing Eof flush emits that release.
        let xml = list_bucket_xml(&["myapp-1.2.3-x86_64-linux"]);
        let releases = super::parse_s3_response(
            &xml,
            "https://bucket.s3.us-east-1.amazonaws.com/",
            #[cfg(feature = "s3-auth")]
            &None,
            #[cfg(feature = "s3-auth")]
            &None,
        )
        .unwrap();
        assert_eq!(releases.len(), 1, "one release parsed");
        let rel = &releases[0];
        assert_eq!(rel.name, "myapp");
        assert_eq!(rel.version, "1.2.3");
        assert_eq!(rel.assets.len(), 1);
        assert_eq!(rel.assets[0].name, "myapp-1.2.3-x86_64-linux");
        assert!(
            rel.assets[0]
                .download_url
                .starts_with("https://bucket.s3.us-east-1.amazonaws.com/"),
            "download URL uses the supplied base"
        );
        assert_eq!(rel.date, "2024-01-01T00:00:00.000Z");
    }

    #[test]
    fn parse_s3_response_v_prefix_stripped() {
        // A `v`-prefixed version tag (e.g. "myapp-v2.0.0-arm-linux") must have the `v` stripped
        // in the parsed release's `version` field, matching the regex's `[v]{0,1}` handling.
        let xml = list_bucket_xml(&["myapp-v2.0.0-arm-linux"]);
        let releases = super::parse_s3_response(
            &xml,
            "https://bucket/",
            #[cfg(feature = "s3-auth")]
            &None,
            #[cfg(feature = "s3-auth")]
            &None,
        )
        .unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version, "2.0.0", "v-prefix must be stripped");
    }

    #[test]
    fn parse_s3_response_multi_asset_merge() {
        // Two <Contents> entries for the same name+version represent two assets of one release.
        // `add_to_releases_list` must merge them into a single release with two assets.
        // The Eof flush handles the last entry, and the interim flush (on the second <Contents>
        // start) handles the first.
        let xml = list_bucket_xml(&["myapp-3.0.0-x86_64-linux", "myapp-3.0.0-aarch64-linux"]);
        let releases = super::parse_s3_response(
            &xml,
            "https://bucket/",
            #[cfg(feature = "s3-auth")]
            &None,
            #[cfg(feature = "s3-auth")]
            &None,
        )
        .unwrap();
        assert_eq!(releases.len(), 1, "same name+version must be merged");
        assert_eq!(
            releases[0].assets.len(),
            2,
            "both assets present after merge"
        );
        let asset_names: Vec<&str> = releases[0].assets.iter().map(|a| a.name.as_str()).collect();
        assert!(
            asset_names.contains(&"myapp-3.0.0-x86_64-linux"),
            "x86_64 asset present"
        );
        assert!(
            asset_names.contains(&"myapp-3.0.0-aarch64-linux"),
            "aarch64 asset present"
        );
    }

    #[test]
    fn parse_s3_response_multiple_releases() {
        // Multiple distinct name/version combinations produce separate release entries.
        // Also exercises the interim <Contents> flush path (not just the Eof flush).
        let xml = list_bucket_xml(&[
            "myapp-1.0.0-x86_64-linux",
            "myapp-2.0.0-x86_64-linux",
            "otherapp-1.5.0-x86_64-linux",
        ]);
        let releases = super::parse_s3_response(
            &xml,
            "https://bucket/",
            #[cfg(feature = "s3-auth")]
            &None,
            #[cfg(feature = "s3-auth")]
            &None,
        )
        .unwrap();
        assert_eq!(releases.len(), 3, "three distinct releases");
        let versions: Vec<&str> = releases.iter().map(|r| r.version.as_str()).collect();
        assert!(versions.contains(&"1.0.0"));
        assert!(versions.contains(&"2.0.0"));
        assert!(versions.contains(&"1.5.0"));
    }

    #[test]
    fn parse_s3_response_skips_non_matching_keys() {
        // Keys that don't match the version regex (no semver-like version component) must be
        // silently ignored; only the matching entry produces a release.
        let xml = list_bucket_xml(&[
            "README.txt",
            "myapp-1.0.0-x86_64-linux",
            "some/random/path/no-version",
        ]);
        let releases = super::parse_s3_response(
            &xml,
            "https://bucket/",
            #[cfg(feature = "s3-auth")]
            &None,
            #[cfg(feature = "s3-auth")]
            &None,
        )
        .unwrap();
        assert_eq!(releases.len(), 1, "only matching key produces a release");
        assert_eq!(releases[0].version, "1.0.0");
    }

    #[test]
    fn parse_s3_response_prefix_path_stripped_to_filename() {
        // When the <Key> contains a directory prefix (e.g. "releases/myapp-1.0.0-linux"),
        // the asset `name` must be just the filename component, not the full path.
        let xml = list_bucket_xml(&["releases/myapp-1.0.0-x86_64-linux"]);
        let releases = super::parse_s3_response(
            &xml,
            "https://bucket/",
            #[cfg(feature = "s3-auth")]
            &None,
            #[cfg(feature = "s3-auth")]
            &None,
        )
        .unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(
            releases[0].assets[0].name, "myapp-1.0.0-x86_64-linux",
            "asset name is the filename, not the full key path"
        );
    }

    #[test]
    fn parse_s3_response_malformed_xml_errors() {
        // A body that is not valid XML must surface as an `Err`, not panic.
        let bad_xml = "this is not xml at all <<<";
        let result = super::parse_s3_response(
            bad_xml,
            "https://bucket/",
            #[cfg(feature = "s3-auth")]
            &None,
            #[cfg(feature = "s3-auth")]
            &None,
        );
        assert!(result.is_err(), "malformed XML must return Err");
    }

    #[test]
    fn parse_s3_response_empty_body_returns_empty_vec() {
        // An empty/minimal XML document with no <Contents> produces an empty releases list (not
        // an error), since there is simply nothing to parse.
        let xml = "<?xml version=\"1.0\"?><ListBucketResult></ListBucketResult>";
        let releases = super::parse_s3_response(
            xml,
            "https://bucket/",
            #[cfg(feature = "s3-auth")]
            &None,
            #[cfg(feature = "s3-auth")]
            &None,
        )
        .unwrap();
        assert!(releases.is_empty(), "empty bucket produces empty list");
    }

    #[test]
    fn add_to_releases_list_skips_entries_with_empty_name_or_version() {
        // `add_to_releases_list` must silently drop a release whose name or version is empty,
        // matching the `if !rel.version.is_empty() && !rel.name.is_empty()` guard.
        let mut releases = Vec::new();
        let empty_name = Release::builder()
            .name("")
            .version("1.0.0")
            .build()
            .unwrap();
        let empty_ver = Release::builder()
            .name("myapp")
            .version("")
            .build()
            .unwrap();
        super::add_to_releases_list(&mut releases, empty_name);
        super::add_to_releases_list(&mut releases, empty_ver);
        assert!(
            releases.is_empty(),
            "entries with empty name or version must be dropped"
        );
    }

    // ---------------------------------------------------------------------------
    // Async-fetch tests via a loopback TCP stub
    // ---------------------------------------------------------------------------

    #[cfg(feature = "async")]
    use std::io::{Read as _, Write as _};
    #[cfg(feature = "async")]
    use std::net::TcpListener;

    /// Serve a single XML response over a loopback TCP listener, one connection per `Resp`.
    /// Returns the base URL (`http://127.0.0.1:<port>/`).
    #[cfg(feature = "async")]
    struct Resp {
        status: &'static str,
        body: String,
    }

    #[cfg(feature = "async")]
    fn stub(responses: Vec<Resp>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for r in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let out = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    r.status,
                    r.body.len(),
                    r.body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        base
    }

    /// Build a `fetch_releases_from_s3_async`-ready `Update` whose `EndPoint::Generic` points at
    /// the stub base URL. The Generic endpoint does not require a region.
    #[cfg(feature = "async")]
    fn s3_update(base_url: &str, current_version: &str) -> Update {
        Update::configure()
            .end_point(super::EndPoint::Generic {
                end_point: base_url.to_owned(),
            })
            .bucket_name("test-bucket")
            .bin_name("myapp")
            .current_version(current_version)
            .build_async()
            .unwrap()
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn fetch_releases_from_s3_async_parses_xml_response() {
        // Drive `fetch_releases_from_s3_async` against a loopback stub that returns a valid S3
        // `ListBucketResult` XML body, and assert the parsed releases.
        let xml = list_bucket_xml(&["myapp-2.1.0-x86_64-linux"]);
        let base = stub(vec![Resp {
            status: "200 OK",
            body: xml,
        }]);
        let upd = s3_update(&base, "0.1.0");
        let rel = upd.get_latest_release_async().await.unwrap();
        assert_eq!(rel.version, "2.1.0");
        assert_eq!(rel.name, "myapp");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_releases_async_filters_to_newer_only() {
        // A ListBucketResult with releases at versions 0.9.0, 1.0.0, 1.5.0, and 2.0.0.
        // With current_version=1.0.0, only 1.5.0 and 2.0.0 should survive (newest-first).
        let xml = list_bucket_xml(&[
            "myapp-0.9.0-x86_64-linux",
            "myapp-1.0.0-x86_64-linux",
            "myapp-1.5.0-x86_64-linux",
            "myapp-2.0.0-x86_64-linux",
        ]);
        let base = stub(vec![Resp {
            status: "200 OK",
            body: xml,
        }]);
        let upd = s3_update(&base, "1.0.0");
        use crate::update::AsyncFetch;
        let releases = upd.get_latest_releases_async("1.0.0").await.unwrap();
        let versions: Vec<&str> = releases.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(
            versions,
            vec!["2.0.0", "1.5.0"],
            "only releases strictly newer than current, newest-first"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_release_version_async_finds_exact_version() {
        // A ListBucketResult with two releases; `get_release_version_async` must return only the
        // one matching the requested version.
        let xml = list_bucket_xml(&["myapp-1.0.0-x86_64-linux", "myapp-2.0.0-x86_64-linux"]);
        let base = stub(vec![Resp {
            status: "200 OK",
            body: xml,
        }]);
        let upd = s3_update(&base, "0.1.0");
        use crate::update::AsyncFetch;
        let rel = upd.get_release_version_async("1.0.0").await.unwrap();
        assert_eq!(rel.version, "1.0.0");
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_release_version_async_errors_on_missing_version() {
        // When the requested version does not exist in the bucket listing, the call must error
        // with `Error::Release`.
        let xml = list_bucket_xml(&["myapp-1.0.0-x86_64-linux"]);
        let base = stub(vec![Resp {
            status: "200 OK",
            body: xml,
        }]);
        let upd = s3_update(&base, "0.1.0");
        use crate::update::AsyncFetch;
        let res = upd.get_release_version_async("9.9.9").await;
        assert!(
            matches!(res, Err(crate::errors::Error::Release(_))),
            "missing version must surface as Error::Release"
        );
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn get_latest_release_async_multi_asset_merge() {
        // Two <Contents> entries for the same name+version are merged into a single release with
        // two assets. `get_latest_release_async` must return that merged release.
        let xml = list_bucket_xml(&["myapp-3.0.0-x86_64-linux", "myapp-3.0.0-aarch64-linux"]);
        let base = stub(vec![Resp {
            status: "200 OK",
            body: xml,
        }]);
        let upd = s3_update(&base, "0.1.0");
        let rel = upd.get_latest_release_async().await.unwrap();
        assert_eq!(rel.version, "3.0.0");
        assert_eq!(
            rel.assets.len(),
            2,
            "both assets present after async multi-asset merge"
        );
    }

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
    fn pick_latest_ignores_unparseable_versions() {
        // `pick_latest` does NOT pre-filter, so its comparator's `Err(_)` branch (unparseable
        // version string) is reachable here. A release with a non-semver version must be ignored
        // and the highest parseable version still chosen. (`choose_latest_release`/`sort_newer`
        // pre-filter unparseable versions, so their comparator `Err(_)` arm is unreachable.)
        let releases = [rel("1.0.0"), rel("not-a-version"), rel("2.1.0")];
        assert_eq!(super::pick_latest(&releases).unwrap().version, "2.1.0");

        // Even when the unparseable one is first/last, it never wins.
        let releases = [rel("bogus"), rel("1.5.0")];
        assert_eq!(super::pick_latest(&releases).unwrap().version, "1.5.0");
    }

    #[test]
    fn sort_newer_ignores_unparseable_versions() {
        // The pre-filter drops the unparseable version before the sort; only parseable, strictly
        // newer versions survive, newest-first.
        let releases = vec![rel("garbage"), rel("2.0.0"), rel("1.5.0"), rel("1.0.0")];
        let newer = super::sort_newer(releases, "1.0.0");
        let versions: Vec<_> = newer.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(versions, vec!["2.0.0", "1.5.0"]);
    }

    #[cfg(feature = "s3-auth")]
    #[test]
    fn access_key_is_reexported_and_built_from_tuples() {
        // `AccessKey` is re-exported at the backend module level and is built via the tuple `From`
        // impls (it is `#[non_exhaustive]`, so no struct literal from outside this module).
        let from_strs: super::AccessKey = ("AKIA-id", "secret").into();
        assert_eq!(from_strs.access_key_id, "AKIA-id");
        assert_eq!(from_strs.secret_access_key, "secret");

        let from_owned: super::AccessKey = (String::from("id2"), String::from("secret2")).into();
        assert_eq!(from_owned.access_key_id, "id2");
        assert_eq!(from_owned.secret_access_key, "secret2");
    }

    #[cfg(feature = "s3-auth")]
    #[test]
    fn access_key_setter_accepts_tuple_and_reexported_type() {
        // The `access_key` setter takes `impl Into<AccessKey>`, so both a bare tuple and an
        // already-built `AccessKey` (named via the re-export) compile and build.
        let _ = Update::configure()
            .bucket_name("bucket")
            .region("us-east-1")
            .bin_name("my_bin")
            .current_version("0.1.0")
            .access_key(("id", "secret"))
            .build()
            .unwrap();

        let key: super::AccessKey = ("id", "secret").into();
        let _ = Update::configure()
            .bucket_name("bucket")
            .region("us-east-1")
            .bin_name("my_bin")
            .current_version("0.1.0")
            .access_key(key)
            .build()
            .unwrap();
    }

    #[test]
    fn filter_target_setter_exists_on_release_list_builder() {
        // The renamed `filter_target` setter must exist on the s3 `ReleaseListBuilder` and the
        // builder must build with the required fields present.
        let _list = super::ReleaseList::configure()
            .bucket_name("bucket")
            .filter_target("x86_64-unknown-linux-gnu")
            .build()
            .unwrap();
    }

    #[test]
    fn release_list_build_surfaces_invalid_header() {
        // A bad header on the `ReleaseListBuilder` must fail at `build()` via `request.check()`
        // with `Error::Config`, not panic.
        let res = super::ReleaseList::configure()
            .bucket_name("bucket")
            .request_header("inva lid", "ok")
            .build();
        assert!(
            matches!(res, Err(crate::errors::Error::Config(_))),
            "invalid header must surface as Error::Config from ReleaseList build()"
        );
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
            .asset_identifier("musl")
            .current_version("0.1.0")
            .build()
            .unwrap()
    }

    #[test]
    fn identifier_and_str_accessors_are_wired() {
        let upd = configured();
        // `identifier` is newly supported on the s3 builder.
        assert_eq!(upd.asset_identifier(), Some("musl"));
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

    // ---------------------------------------------------------------------------
    // build_s3_api_url: endpoint/region/prefix URL construction (pure, no network)
    // ---------------------------------------------------------------------------

    /// Call `build_s3_api_url`, threading the `s3-auth`-only `access_key` argument behind the
    /// feature gate so the same call site compiles with and without `s3-auth`. With no access key
    /// the returned `api_url` is unsigned, so the tests below can assert on the raw URL shape.
    fn api_url(
        end_point: super::EndPoint,
        bucket: &str,
        region: Option<&str>,
        prefix: Option<&str>,
    ) -> crate::errors::Result<(String, String)> {
        super::build_s3_api_url(
            &end_point,
            bucket,
            &region.map(str::to_owned),
            &prefix.map(str::to_owned),
            #[cfg(feature = "s3-auth")]
            &None,
        )
    }

    #[test]
    fn build_s3_api_url_s3_endpoint_shape() {
        // EndPoint::S3 forms `https://<bucket>.s3.<region>.amazonaws.com/` as the download base,
        // and the listing url appends the v2 `list-type=2&max-keys=...` query.
        let (base, url) =
            api_url(super::EndPoint::S3, "my-bucket", Some("eu-west-1"), None).unwrap();
        assert_eq!(base, "https://my-bucket.s3.eu-west-1.amazonaws.com/");
        assert_eq!(
            url,
            "https://my-bucket.s3.eu-west-1.amazonaws.com/?list-type=2&max-keys=100"
        );
    }

    #[test]
    fn build_s3_api_url_dualstack_endpoint_shape() {
        // EndPoint::S3DualStack injects the `dualstack` infix into the host.
        let (base, url) =
            api_url(super::EndPoint::S3DualStack, "b", Some("us-east-2"), None).unwrap();
        assert_eq!(base, "https://b.s3.dualstack.us-east-2.amazonaws.com/");
        assert!(url.starts_with("https://b.s3.dualstack.us-east-2.amazonaws.com/?list-type=2"));
    }

    #[test]
    fn build_s3_api_url_digitalocean_endpoint_shape() {
        // EndPoint::DigitalOceanSpaces uses `<bucket>.<region>.digitaloceanspaces.com`.
        let (base, url) = api_url(
            super::EndPoint::DigitalOceanSpaces,
            "space",
            Some("nyc3"),
            None,
        )
        .unwrap();
        assert_eq!(base, "https://space.nyc3.digitaloceanspaces.com/");
        assert!(url.starts_with("https://space.nyc3.digitaloceanspaces.com/?list-type=2"));
    }

    #[test]
    fn build_s3_api_url_gcs_ignores_region_and_uses_maxkeys_only() {
        // EndPoint::GCS targets `storage.googleapis.com/<bucket>/`, does NOT embed a region, and
        // its listing query is `max-keys` only (no `list-type=2`, which is S3-specific).
        let (base, url) = api_url(super::EndPoint::GCS, "gbucket", None, None).unwrap();
        assert_eq!(base, "https://storage.googleapis.com/gbucket/");
        assert_eq!(url, "https://storage.googleapis.com/gbucket/?max-keys=100");
        assert!(
            !url.contains("list-type=2"),
            "GCS listing must not use the S3-only list-type=2 param"
        );
    }

    #[test]
    fn build_s3_api_url_generic_passes_endpoint_through() {
        // EndPoint::Generic uses the supplied URL verbatim as the download base (region is not
        // consumed) and appends the v2 `list-type=2` listing query.
        let (base, url) = api_url(
            super::EndPoint::Generic {
                end_point: "https://s3.example.com/bucket/".to_owned(),
            },
            "ignored-bucket",
            None,
            None,
        )
        .unwrap();
        assert_eq!(base, "https://s3.example.com/bucket/");
        assert_eq!(
            url,
            "https://s3.example.com/bucket/?list-type=2&max-keys=100"
        );
    }

    #[test]
    fn build_s3_api_url_appends_asset_prefix() {
        // A configured asset_prefix is appended as `&prefix=<value>` to the listing query; with
        // no prefix the segment is absent.
        let (_base, with_prefix) = api_url(
            super::EndPoint::S3,
            "b",
            Some("us-east-1"),
            Some("releases/"),
        )
        .unwrap();
        assert!(
            with_prefix.ends_with("&prefix=releases/"),
            "prefix must be appended: {}",
            with_prefix
        );
        let (_base, no_prefix) =
            api_url(super::EndPoint::S3, "b", Some("us-east-1"), None).unwrap();
        assert!(
            !no_prefix.contains("prefix="),
            "no prefix segment when asset_prefix is None"
        );
    }

    #[test]
    fn build_s3_api_url_missing_region_errors_for_region_endpoints() {
        // S3, S3DualStack and DigitalOceanSpaces all interpolate the region into the host, so a
        // missing region must surface as `Error::Config` (not a panic or a malformed URL).
        for ep in [
            super::EndPoint::S3,
            super::EndPoint::S3DualStack,
            super::EndPoint::DigitalOceanSpaces,
        ] {
            let res = api_url(ep, "b", None, None);
            assert!(
                matches!(res, Err(crate::errors::Error::Config(_))),
                "region-requiring endpoint without region must error with Error::Config"
            );
        }
    }

    #[test]
    fn build_s3_api_url_generic_and_gcs_succeed_without_region() {
        // Generic and GCS never read the region, so both must build successfully when region is
        // absent (the region-requiring endpoints are covered by the error test above).
        assert!(api_url(super::EndPoint::GCS, "b", None, None).is_ok());
        assert!(api_url(
            super::EndPoint::Generic {
                end_point: "https://s3.example.com/".to_owned()
            },
            "b",
            None,
            None
        )
        .is_ok());
    }

    // ---------------------------------------------------------------------------
    // s3-auth: SigV4 query-signing structural invariants (deterministic, no real
    // signature assertions since the timestamp/HMAC are time-dependent)
    // ---------------------------------------------------------------------------

    #[cfg(feature = "s3-auth")]
    #[test]
    fn s3_signature_v4_no_access_key_returns_url_unchanged() {
        // With no access key the signer is a no-op: the URL is returned verbatim, so an
        // unauthenticated (public-bucket) request carries no SigV4 query params.
        let url = "https://b.s3.us-east-1.amazonaws.com/key?list-type=2";
        let out = super::auth::s3_signature_v4(url, &Some("us-east-1".into()), &None, 300).unwrap();
        assert_eq!(out, url, "missing access key must return the URL unchanged");
    }

    #[cfg(feature = "s3-auth")]
    #[test]
    fn s3_signature_v4_signed_url_has_required_query_params() {
        // With an access key the signer appends the AWS SigV4 presigned-query params. We cannot
        // assert the exact signature (it depends on the current wall-clock timestamp and HMAC),
        // but the structural invariants are deterministic.
        let url = "https://b.s3.us-east-1.amazonaws.com/path/to/key";
        let key: super::AccessKey = ("AKIAEXAMPLE", "secretkey").into();
        let out =
            super::auth::s3_signature_v4(url, &Some("us-east-1".into()), &Some(key), 300).unwrap();
        assert!(out.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(out.contains("X-Amz-Credential="));
        assert!(out.contains("X-Amz-Date="));
        assert!(out.contains("X-Amz-Expires=300"));
        assert!(out.contains("X-Amz-SignedHeaders=host"));
        assert!(out.contains("X-Amz-Signature="));
        // The signature is the final param and is a lowercase hex SHA-256 HMAC (64 hex chars).
        let sig = out
            .rsplit("X-Amz-Signature=")
            .next()
            .expect("signature param present");
        assert_eq!(sig.len(), 64, "SigV4 signature is 32 bytes hex-encoded");
        assert!(
            sig.bytes().all(|b| b.is_ascii_hexdigit()),
            "signature must be lowercase hex"
        );
        // The credential scope embeds the region and the s3/aws4_request terminator. The whole
        // X-Amz-Credential value is URI-encoded in the query string, so the scope separators are
        // percent-encoded slashes (`%2F`).
        assert!(
            out.contains("%2Fus-east-1%2Fs3%2Faws4_request"),
            "credential scope must embed region and service scope: {}",
            out
        );
        // The base path is preserved ahead of the query string.
        assert!(out.starts_with("https://b.s3.us-east-1.amazonaws.com/path/to/key?"));
    }

    #[cfg(feature = "s3-auth")]
    #[test]
    fn s3_signature_v4_defaults_region_to_us_east_1() {
        // When region is None the signer falls back to `us-east-1` in the credential scope.
        let key: super::AccessKey = ("AKIA", "secret").into();
        let out =
            super::auth::s3_signature_v4("https://b.s3.amazonaws.com/k", &None, &Some(key), 300)
                .unwrap();
        assert!(
            out.contains("%2Fus-east-1%2Fs3%2Faws4_request"),
            "absent region must default to us-east-1: {}",
            out
        );
    }

    #[cfg(feature = "s3-auth")]
    #[test]
    fn build_s3_api_url_signs_listing_url_when_access_key_present() {
        // The listing url returned by `build_s3_api_url` is signed when an access key is supplied,
        // and left unsigned otherwise. This exercises the s3-auth branch at the call site (not
        // just the bare signer), preserving the existing `list-type=2` query under signing.
        let key: super::AccessKey = ("AKIA", "secret").into();
        let region = Some("us-east-1".to_owned());
        let (_base, signed) =
            super::build_s3_api_url(&super::EndPoint::S3, "b", &region, &None, &Some(key)).unwrap();
        assert!(signed.contains("X-Amz-Signature="));
        assert!(signed.contains("X-Amz-Credential="));
        assert!(
            signed.contains("list-type=2"),
            "the original listing query must survive signing"
        );

        let (_base, unsigned) =
            super::build_s3_api_url(&super::EndPoint::S3, "b", &region, &None, &None).unwrap();
        assert!(
            !unsigned.contains("X-Amz-Signature="),
            "no access key => unsigned listing url"
        );
    }

    #[cfg(feature = "s3-auth")]
    #[test]
    fn parse_s3_response_signs_download_urls_when_access_key_present() {
        // Under s3-auth, `parse_s3_response` signs each asset's download URL. The unsigned vs
        // signed distinction is the load-bearing behavior: with an access key the asset URL gains
        // the SigV4 presign params; with none it stays a plain URL.
        let xml = list_bucket_xml(&["myapp-1.2.3-x86_64-linux"]);
        let region = Some("us-east-1".to_owned());

        let key: super::AccessKey = ("AKIA", "secret").into();
        let signed = super::parse_s3_response(
            &xml,
            "https://b.s3.us-east-1.amazonaws.com/",
            &region,
            &Some(key),
        )
        .unwrap();
        let signed_url = &signed[0].assets[0].download_url;
        assert!(
            signed_url.contains("X-Amz-Signature=") && signed_url.contains("X-Amz-Credential="),
            "signed asset download url must carry SigV4 params: {}",
            signed_url
        );

        let unsigned = super::parse_s3_response(
            &xml,
            "https://b.s3.us-east-1.amazonaws.com/",
            &region,
            &None,
        )
        .unwrap();
        let unsigned_url = &unsigned[0].assets[0].download_url;
        assert!(
            !unsigned_url.contains("X-Amz-Signature="),
            "no access key => unsigned asset download url: {}",
            unsigned_url
        );
        assert_eq!(
            unsigned_url, "https://b.s3.us-east-1.amazonaws.com/myapp-1.2.3-x86_64-linux",
            "unsigned download url is the plain base+key"
        );
    }

    #[test]
    fn api_headers_uses_the_trait_default_no_override() {
        // s3 passes NO `{api_headers}` override to `impl_update_config_accessors!`, so it must get
        // the `UpdateConfig` trait default: a single `token` Authorization header and, crucially,
        // *no* User-Agent (unlike github/gitlab/gitea which override with one).
        let upd = configured();
        let headers = upd.api_headers(Some("secret")).unwrap();
        assert!(
            headers
                .get(crate::http_client::header::USER_AGENT)
                .is_none(),
            "the default api_headers (no override) must not set a User-Agent"
        );
        assert_eq!(
            headers
                .get(crate::http_client::header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "token secret"
        );
    }
}

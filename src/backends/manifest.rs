/*!
Updates from a static JSON release manifest served over HTTP(S).

Use this backend to update from any plain file server (S3 static hosting, a CDN, GitHub Pages, a
bare nginx, ...) that serves a single JSON document describing your releases, plus the release
artifacts alongside it. It is the service-agnostic fallback for hosts the forge backends
(`github`, `gitlab`, `gitea`, `s3`) don't cover, and needs no server-side API, just a static
`manifest.json` you regenerate at release time.

The backend fetches the manifest, parses it into [`Release`]s, and drives the crate's usual
compare -> select-asset -> download -> verify -> extract -> install flow (via the same pipeline
the [`custom`](crate::backends::custom) backend uses).

```no_run
# fn run() -> Result<(), Box<dyn std::error::Error>> {
use self_update::cargo_crate_version;

let status = self_update::backends::manifest::Update::configure()
    .manifest_url("https://example.net/releases/manifest.json")
    .bin_name("app")
    .current_version(cargo_crate_version!())
    .build()?
    .update()?;
println!("update status: `{}`", status.version());
# Ok(())
# }
```

# Manifest schema

The manifest is a JSON object with a `schema` version and a list of `releases`. Unknown fields are
ignored (forward compatibility), and `date`, `notes_url`, and per-asset `digest` are optional.

```json
{
  "schema": 1,
  "releases": [
    {
      "version": "1.2.3",
      "date": "2026-07-16T00:00:00Z",
      "notes_url": "https://example.net/releases/1.2.3",
      "assets": [
        {
          "name": "app-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
          "url": "app-1.2.3-x86_64-unknown-linux-gnu.tar.gz",
          "digest": "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        }
      ]
    }
  ]
}
```

- `schema` is the manifest format version. This crate supports schema `1`; a manifest declaring
  any other `schema` fails with [`Error::InvalidResponse`](crate::errors::Error::InvalidResponse)
  naming the version it found, so an old client refuses a manifest it can't understand rather than
  silently mis-parsing it.
- Each release's `version` must be a bare semver string (no leading `v`). A release whose version
  is not valid semver is **skipped** (logged at `debug`), not an error, so a manifest mixing a
  rolling `nightly` entry with real releases stays usable, matching how the forge backends skip
  non-semver tags.
- An asset `url` may be **relative**: it is resolved against the manifest URL's directory (the URL
  up to and including its last `/`). An absolute `http(s)://` URL is used verbatim. `..` path
  segments are not specially handled.
- `digest` (in `algorithm:hex` form, e.g. `sha256:...`) is mapped onto the asset and, with the
  `checksums` feature, verified against the downloaded artifact before installing (see
  `verify_release_digest` on the builder).

# Async

With the `async` feature, [`build_async`](UpdateBuilder::build_async) returns an [`AsyncUpdate`]
whose `*_async` verbs run the same flow asynchronously.
*/

use std::sync::Arc;

use crate::backends::common::{CommonBuilderConfig, CommonConfig, RequestConfig};
#[cfg(feature = "async")]
use crate::backends::send_async;
use crate::backends::{MAX_LISTING_BODY_BYTES, send};
use crate::errors::*;
use crate::http_client;
use crate::update::{Release, ReleaseAsset, ReleaseSource, ReleaseUpdate, Releases};

/// The manifest `schema` version this crate understands. A manifest declaring any other
/// version is rejected (see [`parse_manifest`]).
const MANIFEST_SCHEMA_VERSION: u64 = 1;

// --- Manifest schema (serde) -------------------------------------------------------------------
//
// No `deny_unknown_fields`: unknown fields are ignored so a newer manifest producer can add fields
// without breaking older clients (the `schema` bump is the explicit break signal instead).

#[derive(serde::Deserialize)]
struct Manifest {
    schema: u64,
    #[serde(default)]
    releases: Vec<ManifestRelease>,
}

#[derive(serde::Deserialize)]
struct ManifestRelease {
    version: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    notes_url: Option<String>,
    #[serde(default)]
    assets: Vec<ManifestAsset>,
}

#[derive(serde::Deserialize)]
struct ManifestAsset {
    name: String,
    url: String,
    #[serde(default)]
    digest: Option<String>,
}

/// Resolve an asset URL against the manifest URL.
///
/// An absolute `http(s)://` URL passes through verbatim. A relative URL is joined onto the
/// manifest URL's "directory": everything up to and including the manifest URL's last `/`. This is
/// a deliberately simple string join (no `url` crate, which is an optional s3-only dependency), so
/// `..` segments are NOT collapsed and a manifest URL carrying a query string with a `/` in it
/// would split at that `/`. Point `manifest_url` at a plain path ending in the manifest file name
/// and keep asset URLs as sibling file names (or absolute URLs) to stay in the well-defined case.
fn resolve_asset_url(manifest_url: &str, asset_url: &str) -> String {
    if asset_url.starts_with("http://") || asset_url.starts_with("https://") {
        return asset_url.to_string();
    }
    match manifest_url.rfind('/') {
        // `..=idx` keeps the trailing '/', so "https://h/a/manifest.json" + "app.tgz"
        // -> "https://h/a/app.tgz".
        Some(idx) => format!("{}{}", &manifest_url[..=idx], asset_url),
        // No '/' at all (degenerate): fall back to the asset URL as given.
        None => asset_url.to_string(),
    }
}

/// Parse a JSON release manifest body into [`Release`]s. Transport-free, so it is shared by the
/// sync and async fetch paths and exercised directly by the unit tests.
///
/// `manifest_url` is used only to resolve relative asset URLs (see [`resolve_asset_url`]).
///
/// * Errors
///     * [`Error::InvalidResponse`](crate::errors::Error::InvalidResponse) if the body is not the
///       expected JSON (including a missing required field), or if the manifest declares a
///       `schema` other than the version this crate supports.
///
/// A release whose `version` is not valid semver is skipped (logged at `debug`), not an error.
pub fn parse_manifest(body: &str, manifest_url: &str) -> Result<Vec<Release>> {
    let manifest: Manifest = serde_json::from_str(body).map_err(Error::invalid_response)?;
    if manifest.schema != MANIFEST_SCHEMA_VERSION {
        return Err(Error::invalid_response(format!(
            "unsupported manifest schema version {}; this crate supports schema {}",
            manifest.schema, MANIFEST_SCHEMA_VERSION
        )));
    }

    let mut releases = Vec::new();
    for mr in manifest.releases {
        let mut builder = Release::builder();
        builder.version(&mr.version);
        if let Some(date) = &mr.date {
            builder.date(date);
        }
        if let Some(notes_url) = &mr.notes_url {
            builder.release_notes_url(notes_url);
        }
        for asset in &mr.assets {
            let url = resolve_asset_url(manifest_url, &asset.url);
            let mut release_asset = ReleaseAsset::new(&*asset.name, url);
            if let Some(digest) = &asset.digest {
                release_asset = release_asset.with_digest(&**digest);
            }
            builder.asset(release_asset);
        }
        match builder.build() {
            Ok(release) => releases.push(release),
            // A non-semver version is not a release the updater can compare; skip it rather than
            // failing the whole manifest, so a manifest mixing rolling entries with real releases
            // stays updatable. Matches the forge backends' skip-non-semver-tags precedent (#190).
            Err(e @ Error::SemVer(_)) => {
                log::debug!("self_update: skipping manifest release: {e}");
            }
            Err(e) => return Err(e),
        }
    }
    Ok(releases)
}

/// Base headers sent with the manifest fetch (a user `request_header(..)` merges on top of these).
fn base_headers() -> http_client::HeaderMap {
    let mut headers = http_client::HeaderMap::new();
    headers.insert(
        http_client::header::ACCEPT,
        http_client::header::HeaderValue::from_static("application/json"),
    );
    headers
}

/// Read a manifest response body into a `String`, bounded by [`MAX_LISTING_BODY_BYTES`] so a
/// misconfigured or malicious endpoint cannot force unbounded memory use.
fn read_body(resp: Box<dyn http_client::HttpResponse>) -> Result<String> {
    use std::io::Read as _;
    // Read one byte past the cap to distinguish "exactly at the cap" (fine) from "over it" (error).
    let mut limited = resp.body().take((MAX_LISTING_BODY_BYTES + 1) as u64);
    let mut body = Vec::new();
    limited.read_to_end(&mut body)?;
    if body.len() > MAX_LISTING_BODY_BYTES {
        return Err(Error::invalid_response(format!(
            "manifest body exceeded the {MAX_LISTING_BODY_BYTES}-byte cap; a release manifest is \
             much smaller"
        )));
    }
    String::from_utf8(body)
        .map_err(|e| Error::invalid_response(format!("manifest was not valid UTF-8: {e}")))
}

/// A [`ReleaseSource`] that fetches a JSON release manifest from `url`.
///
/// This is the source the [`Update`] facade wraps, but it can also be used directly with the
/// [`custom`](crate::backends::custom) backend (`custom::Update::configure().source(..)`) when you
/// want the manifest source with the custom builder's surface. The transport setters
/// ([`timeout`](Self::timeout), [`request_header`](Self::request_header),
/// [`retries`](Self::retries), ...) configure the manifest fetch.
#[derive(Debug, Clone)]
pub struct ManifestSource {
    url: String,
    request: RequestConfig,
}

impl ManifestSource {
    /// Construct a source that fetches the manifest at `url`.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            request: RequestConfig::default(),
        }
    }

    request_config_setters!(request);

    /// Fetch and parse the manifest (shared by the sync `ReleaseSource` and, indirectly, the async
    /// impl builds its own request the same way).
    fn resolved_request(&self) -> Result<RequestConfig> {
        // Materialize a client from any custom root CA certs (no-op if none / a client was
        // injected) and surface any deferred header/cert error, mirroring the builder's `build()`.
        let mut request = self.request.clone();
        request.build_client();
        request.check()?;
        Ok(request)
    }
}

impl ReleaseSource for ManifestSource {
    fn get_releases(&self) -> Result<Vec<Release>> {
        let request = self.resolved_request()?;
        let resp = send(&self.url, base_headers(), &request)?;
        let body = read_body(resp)?;
        parse_manifest(&body, &self.url)
    }
}

#[cfg(feature = "async")]
impl crate::update::AsyncReleaseSource for ManifestSource {
    async fn get_releases(&self) -> Result<Vec<Release>> {
        let request = self.resolved_request()?;
        let resp = send_async(&self.url, base_headers(), &request).await?;
        let body = resp.text().await?;
        parse_manifest(&body, &self.url)
    }
}

/// [`manifest::Update`](Update) builder.
///
/// Mirrors the [`custom`](crate::backends::custom) backend's builder (it wraps the same update
/// pipeline over a [`ManifestSource`]), adding the [`manifest_url`](Self::manifest_url) setter. The
/// shared transport setters ([`timeout`](Self::timeout), [`request_header`](Self::request_header),
/// [`retries`](Self::retries), an injected client, ...) apply to **both** the manifest fetch and the
/// crate-controlled asset download.
#[must_use]
#[derive(Clone, Debug, Default)]
pub struct UpdateBuilder {
    manifest_url: Option<String>,
    common: CommonBuilderConfig,
}

impl UpdateBuilder {
    /// Initialize a new builder.
    pub fn new() -> Self {
        Default::default()
    }

    /// Set the URL of the JSON release manifest. Required.
    pub fn manifest_url(&mut self, url: impl Into<String>) -> &mut Self {
        self.manifest_url = Some(url.into());
        self
    }

    impl_common_builder_setters!(no_auth_token);

    fn build_update(&self) -> Result<Update> {
        let url = self.manifest_url.clone().ok_or(Error::MissingField {
            field: "manifest_url",
        })?;
        let common = self.common.build()?;
        // Thread the same resolved transport config into the manifest fetch as the download uses,
        // so `.timeout()` / `.request_header()` / `.retries()` / an injected client apply to both.
        let source = ManifestSource {
            url,
            request: common.request.clone(),
        };
        Ok(Update {
            source: Arc::new(source),
            common,
        })
    }

    /// Confirm config and create a ready-to-use [`Update`].
    ///
    /// Returns the concrete [`Update`], which is `Send` and exposes the update verbs as inherent
    /// methods.
    ///
    /// * Errors:
    ///     * `MissingField` - no `manifest_url` was set, or an invalid `Update` configuration
    pub fn build(&self) -> Result<Update> {
        self.build_update()
    }

    /// Confirm config and create a ready-to-use [`AsyncUpdate`] for the async API
    /// (`update_async`).
    ///
    /// Unlike [`build`](Self::build) this returns the distinct [`AsyncUpdate`] newtype, which
    /// exposes only the inherent `*_async` verbs, so a stray blocking `.update()` on an async-built
    /// updater is a compile error rather than a silent block of the executor.
    #[cfg(feature = "async")]
    pub fn build_async(&self) -> Result<AsyncUpdate> {
        Ok(AsyncUpdate(self.build_update()?))
    }
}

/// Updates to a specified or latest release described by a JSON manifest.
#[derive(Debug)]
#[non_exhaustive]
pub struct Update {
    source: Arc<ManifestSource>,
    common: CommonConfig,
}

impl Update {
    /// Initialize a new `Update` builder.
    pub fn configure() -> UpdateBuilder {
        UpdateBuilder::new()
    }
}

impl crate::update::sealed::Sealed for Update {}

impl_update_config_accessors!(Update);

impl ReleaseUpdate for Update {
    fn get_latest_release(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let release = self.source.get_latest_release()?;
        Ok(Releases::new(vec![release], current_version))
    }

    fn get_newer_releases(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = self
            .source
            .get_releases()?
            .into_iter()
            .filter(|r| {
                crate::version::bump_is_greater(&current_version, r.version()).unwrap_or(false)
            })
            .collect();
        Ok(Releases::new(releases, current_version))
    }

    fn get_release_version(&self, ver: &str) -> Result<Release> {
        self.source.get_release_version(ver)
    }
}

impl_sync_update_verbs!(Update);

/// Async-only updater returned by [`UpdateBuilder::build_async`].
///
/// A newtype over the blocking [`Update`] that exposes **only** the inherent `*_async` verbs, so a
/// blocking call on an async-built updater (e.g. `build_async()?.update()`) is a compile error.
#[cfg(feature = "async")]
#[derive(Debug)]
pub struct AsyncUpdate(Update);

#[cfg(feature = "async")]
impl_async_update_verbs!(AsyncUpdate);

#[cfg(feature = "async")]
impl crate::update::AsyncReleaseUpdate for Update {
    async fn get_latest_release_async(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let release = crate::update::AsyncReleaseSource::get_latest_release(&*self.source).await?;
        Ok(Releases::new(vec![release], current_version))
    }

    async fn get_newer_releases_async(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = crate::update::AsyncReleaseSource::get_releases(&*self.source)
            .await?
            .into_iter()
            .filter(|r| {
                crate::version::bump_is_greater(&current_version, r.version()).unwrap_or(false)
            })
            .collect();
        Ok(Releases::new(releases, current_version))
    }

    async fn get_release_version_async(&self, ver: &str) -> Result<Release> {
        crate::update::AsyncReleaseSource::get_release_version(&*self.source, ver).await
    }
}

#[cfg(test)]
mod tests {
    use super::{ManifestSource, Update, parse_manifest, resolve_asset_url};
    use crate::update::ReleaseSource;

    const MANIFEST_URL: &str = "https://example.net/releases/manifest.json";

    // --- parse_manifest unit tests (transport-free) --------------------------------------------

    #[test]
    fn parse_manifest_schema_too_new_errors_naming_the_version() {
        // A manifest declaring a schema newer than supported must be refused (not silently
        // mis-parsed), and the error must name the version found.
        let body = r#"{ "schema": 2, "releases": [] }"#;
        let err = parse_manifest(body, MANIFEST_URL)
            .expect_err("schema 2 must be rejected by a schema-1 client");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "schema-too-new must be InvalidResponse, got {err:?}"
        );
        let shown = err.to_string();
        assert!(
            shown.contains("unsupported manifest schema version 2"),
            "the error must name the found schema version 2: {shown}"
        );
    }

    #[test]
    fn parse_manifest_missing_version_field_errors() {
        // A release object without the required `version` field is a parse error.
        let body = r#"{ "schema": 1, "releases": [ { "assets": [] } ] }"#;
        let err = parse_manifest(body, MANIFEST_URL).expect_err("missing `version` must error");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "a missing required field must be InvalidResponse, got {err:?}"
        );
    }

    #[test]
    fn parse_manifest_missing_asset_name_errors() {
        let body = r#"{ "schema": 1, "releases": [
            { "version": "1.0.0", "assets": [ { "url": "app.tar.gz" } ] } ] }"#;
        let err = parse_manifest(body, MANIFEST_URL).expect_err("missing asset `name` must error");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "a missing asset name must be InvalidResponse, got {err:?}"
        );
    }

    #[test]
    fn parse_manifest_missing_asset_url_errors() {
        let body = r#"{ "schema": 1, "releases": [
            { "version": "1.0.0", "assets": [ { "name": "app.tar.gz" } ] } ] }"#;
        let err = parse_manifest(body, MANIFEST_URL).expect_err("missing asset `url` must error");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "a missing asset url must be InvalidResponse, got {err:?}"
        );
    }

    #[test]
    fn parse_manifest_relative_asset_url_resolved_against_manifest_dir() {
        let body = r#"{ "schema": 1, "releases": [
            { "version": "1.2.3", "assets": [
                { "name": "app-1.2.3.tar.gz", "url": "app-1.2.3.tar.gz" } ] } ] }"#;
        let releases = parse_manifest(body, MANIFEST_URL).unwrap();
        assert_eq!(releases.len(), 1);
        let asset = &releases[0].assets()[0];
        assert_eq!(
            asset.download_url(),
            "https://example.net/releases/app-1.2.3.tar.gz",
            "a relative asset url must resolve against the manifest URL's directory"
        );
    }

    #[test]
    fn parse_manifest_absolute_asset_url_passes_through() {
        let body = r#"{ "schema": 1, "releases": [
            { "version": "1.2.3", "assets": [
                { "name": "app.tar.gz", "url": "https://cdn.example.com/app.tar.gz" } ] } ] }"#;
        let releases = parse_manifest(body, MANIFEST_URL).unwrap();
        assert_eq!(
            releases[0].assets()[0].download_url(),
            "https://cdn.example.com/app.tar.gz",
            "an absolute http(s) asset url must pass through verbatim"
        );
    }

    #[test]
    fn resolve_asset_url_handles_relative_absolute_and_no_slash() {
        assert_eq!(
            resolve_asset_url("https://h/a/b/manifest.json", "app.tar.gz"),
            "https://h/a/b/app.tar.gz"
        );
        assert_eq!(
            resolve_asset_url("https://h/a/manifest.json", "http://other/app.tar.gz"),
            "http://other/app.tar.gz"
        );
        assert_eq!(
            resolve_asset_url("https://h/a/manifest.json", "https://other/app.tar.gz"),
            "https://other/app.tar.gz"
        );
        // Degenerate: a manifest "url" with no slash falls back to the asset url as given.
        assert_eq!(
            resolve_asset_url("manifest.json", "app.tar.gz"),
            "app.tar.gz"
        );
    }

    #[test]
    fn parse_manifest_digest_mapped_through_to_asset() {
        let digest = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let body = format!(
            r#"{{ "schema": 1, "releases": [
                {{ "version": "1.0.0", "assets": [
                    {{ "name": "app.tar.gz", "url": "app.tar.gz", "digest": "{digest}" }} ] }} ] }}"#
        );
        let releases = parse_manifest(&body, MANIFEST_URL).unwrap();
        assert_eq!(
            releases[0].assets()[0].digest(),
            Some(digest),
            "an asset digest must be mapped onto the ReleaseAsset"
        );
    }

    #[test]
    fn parse_manifest_no_digest_leaves_asset_digest_none() {
        let body = r#"{ "schema": 1, "releases": [
            { "version": "1.0.0", "assets": [ { "name": "app.tar.gz", "url": "app.tar.gz" } ] } ] }"#;
        let releases = parse_manifest(body, MANIFEST_URL).unwrap();
        assert_eq!(releases[0].assets()[0].digest(), None);
    }

    #[test]
    fn parse_manifest_date_and_notes_url_mapped() {
        let body = r#"{ "schema": 1, "releases": [
            { "version": "1.0.0", "date": "2026-07-16T00:00:00Z",
              "notes_url": "https://example.net/releases/1.0.0", "assets": [] } ] }"#;
        let releases = parse_manifest(body, MANIFEST_URL).unwrap();
        assert_eq!(releases[0].date(), "2026-07-16T00:00:00Z");
        assert_eq!(
            releases[0].release_notes_url(),
            Some("https://example.net/releases/1.0.0")
        );
    }

    #[test]
    fn parse_manifest_non_semver_version_skipped_valid_siblings_survive() {
        // A non-semver `version` (a rolling `nightly` entry) is skipped, not an error; the valid
        // sibling release survives.
        let body = r#"{ "schema": 1, "releases": [
            { "version": "nightly", "assets": [] },
            { "version": "1.2.3", "assets": [] } ] }"#;
        let releases = parse_manifest(body, MANIFEST_URL).unwrap();
        assert_eq!(releases.len(), 1, "the non-semver release must be skipped");
        assert_eq!(releases[0].version(), "1.2.3");
    }

    #[test]
    fn parse_manifest_unknown_fields_ignored() {
        // Unknown fields at the top level, on a release, and on an asset must be ignored (forward
        // compat), not rejected.
        let body = r#"{
            "schema": 1,
            "generator": "some-tool",
            "releases": [
                { "version": "1.2.3", "channel": "stable", "assets": [
                    { "name": "app.tar.gz", "url": "app.tar.gz", "size": 12345 } ] } ]
        }"#;
        let releases = parse_manifest(body, MANIFEST_URL).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version(), "1.2.3");
        assert_eq!(releases[0].assets().len(), 1);
    }

    // --- resolve_asset_url edge cases (spec: deliberate string-join, no `..`/query handling) -----

    #[test]
    fn resolve_asset_url_trailing_slash_manifest_url_appends_directly() {
        // A manifest URL ending in `/` (a "directory" URL) resolves the asset right after it.
        assert_eq!(
            resolve_asset_url("https://h/a/b/", "app.tar.gz"),
            "https://h/a/b/app.tar.gz"
        );
    }

    #[test]
    fn resolve_asset_url_relative_asset_with_subpath_joins_verbatim() {
        // A relative asset url that itself contains `/` is appended verbatim onto the manifest dir.
        assert_eq!(
            resolve_asset_url("https://h/a/manifest.json", "sub/app.tar.gz"),
            "https://h/a/sub/app.tar.gz"
        );
    }

    #[test]
    fn resolve_asset_url_dotdot_segments_not_collapsed() {
        // Spec: `..` segments are NOT specially handled; they land in the resolved URL as-is.
        assert_eq!(
            resolve_asset_url("https://h/a/b/manifest.json", "../app.tar.gz"),
            "https://h/a/b/../app.tar.gz"
        );
    }

    #[test]
    fn resolve_asset_url_query_string_with_slash_splits_at_that_slash() {
        // Documented degenerate case: the join truncates at the LAST `/`, even one inside a query
        // string. A manifest URL whose query contains a `/` therefore resolves oddly -- this pins
        // that documented behavior so a "fix" that adds query awareness is a conscious change.
        assert_eq!(
            resolve_asset_url("https://h/a/manifest.json?path=/x", "app.tar.gz"),
            "https://h/a/manifest.json?path=/app.tar.gz"
        );
    }

    #[test]
    fn resolve_asset_url_query_string_without_slash_resolves_against_dir() {
        // A query string with no `/` in it does not move the split point: the last `/` is still the
        // one before the manifest file name.
        assert_eq!(
            resolve_asset_url("https://h/a/manifest.json?token=abc", "app.tar.gz"),
            "https://h/a/app.tar.gz"
        );
    }

    // --- parse_manifest schema / body edge cases -----------------------------------------------

    #[test]
    fn parse_manifest_missing_schema_field_errors() {
        // `schema` is a required field (no serde default): a manifest that omits it entirely is a
        // parse error, not silently treated as schema 0/1.
        let body = r#"{ "releases": [] }"#;
        let err = parse_manifest(body, MANIFEST_URL).expect_err("absent `schema` must error");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "an absent schema must be InvalidResponse, got {err:?}"
        );
    }

    #[test]
    fn parse_manifest_schema_wrong_type_errors() {
        // A `schema` of the wrong JSON type (string instead of integer) is a parse error.
        let body = r#"{ "schema": "1", "releases": [] }"#;
        let err = parse_manifest(body, MANIFEST_URL).expect_err("string `schema` must error");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "a wrong-typed schema must be InvalidResponse, got {err:?}"
        );
    }

    #[test]
    fn parse_manifest_garbage_body_errors() {
        // A body that is not JSON at all surfaces as InvalidResponse, never a panic or empty list.
        let err =
            parse_manifest("this is not json", MANIFEST_URL).expect_err("non-JSON body must error");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "a non-JSON body must be InvalidResponse, got {err:?}"
        );
    }

    #[test]
    fn parse_manifest_empty_releases_yields_empty_vec() {
        // An explicitly empty releases array parses cleanly to zero releases (the NoReleaseFound
        // surfaces later, from the update-selection helpers -- see the stub test below).
        let releases = parse_manifest(r#"{ "schema": 1, "releases": [] }"#, MANIFEST_URL).unwrap();
        assert!(releases.is_empty());
    }

    #[test]
    fn parse_manifest_release_with_empty_assets_is_kept_with_zero_assets() {
        // A valid-version release carrying zero assets is a real release (asset selection fails
        // later, but parsing keeps it); the entry must survive with an empty asset list.
        let body = r#"{ "schema": 1, "releases": [ { "version": "1.2.3", "assets": [] } ] }"#;
        let releases = parse_manifest(body, MANIFEST_URL).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].version(), "1.2.3");
        assert!(releases[0].assets().is_empty());
    }

    #[test]
    fn parse_manifest_duplicate_versions_are_all_kept() {
        // The parser does not de-duplicate: two entries with the same version both survive, in
        // document order. (Selection later picks by semver; a tie takes the earliest-positioned.)
        let body = r#"{ "schema": 1, "releases": [
            { "version": "1.2.3", "assets": [ { "name": "a.tgz", "url": "a.tgz" } ] },
            { "version": "1.2.3", "assets": [ { "name": "b.tgz", "url": "b.tgz" } ] } ] }"#;
        let releases = parse_manifest(body, MANIFEST_URL).unwrap();
        assert_eq!(
            releases.len(),
            2,
            "duplicate versions must not be collapsed"
        );
        assert_eq!(releases[0].assets()[0].name(), "a.tgz");
        assert_eq!(releases[1].assets()[0].name(), "b.tgz");
    }

    #[test]
    fn parse_manifest_non_sha256_digest_prefix_is_mapped_verbatim() {
        // Any `digest` string is mapped verbatim through `ReleaseAsset::with_digest`: an
        // unsupported algorithm (e.g. `md5:`) then hard-errors at verify time under the
        // `checksums` feature rather than being silently dropped. Silently ignoring a digest the
        // manifest author supplied would skip verification the user believes is happening.
        let body = r#"{ "schema": 1, "releases": [
            { "version": "1.0.0", "assets": [
                { "name": "app.tar.gz", "url": "app.tar.gz", "digest": "md5:abc123" } ] } ] }"#;
        let releases = parse_manifest(body, MANIFEST_URL).unwrap();
        assert_eq!(
            releases[0].assets()[0].digest(),
            Some("md5:abc123"),
            "current impl maps a non-sha256 digest verbatim (diverges from the spec's \
             treat-as-absent contract)"
        );
    }

    #[test]
    fn parse_manifest_schema_zero_is_rejected() {
        // Only `schema: 1` is accepted: any other value (0 included) is refused with an error
        // naming the received version, per the spec's invariant checklist.
        let err = parse_manifest(r#"{ "schema": 0, "releases": [] }"#, MANIFEST_URL)
            .expect_err("schema 0 must be rejected; only schema 1 is supported");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "schema 0 must be InvalidResponse, got {err:?}"
        );
        assert!(
            err.to_string()
                .contains("unsupported manifest schema version 0"),
            "the error must name the found schema version 0: {err}"
        );
    }

    #[test]
    fn build_requires_a_manifest_url() {
        // Absent `manifest_url` must be the specific `MissingField { field: "manifest_url" }`, and
        // it must take precedence even when the common fields (bin_name/current_version) are set.
        let err = Update::configure()
            .bin_name("app")
            .current_version("1.0.0")
            .build()
            .expect_err("build must fail without a manifest_url");
        assert!(
            matches!(
                err,
                crate::errors::Error::MissingField {
                    field: "manifest_url"
                }
            ),
            "a missing manifest_url must be MissingField {{ field: \"manifest_url\" }}, got {err:?}"
        );
    }

    // --- Loopback stub end-to-end tests --------------------------------------------------------

    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// Serve a sequence of raw HTTP responses over a loopback listener, one connection per entry.
    /// Each entry is `(content_type, body_bytes)`. Returns the base URL (`http://127.0.0.1:<port>`).
    fn stub(responses: Vec<(&'static str, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let owned: Vec<(String, Vec<u8>)> = responses
            .into_iter()
            .map(|(ct, body)| (ct.to_string(), body))
            .collect();
        std::thread::spawn(move || {
            for (content_type, body) in owned {
                let (mut stream, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    content_type,
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        base
    }

    #[test]
    fn get_releases_over_the_stub_parses_the_manifest() {
        let manifest = r#"{ "schema": 1, "releases": [
            { "version": "2.0.0", "assets": [
                { "name": "app-2.0.0.tar.gz", "url": "app-2.0.0.tar.gz" } ] },
            { "version": "1.0.0", "assets": [] } ] }"#;
        let base = stub(vec![("application/json", manifest.as_bytes().to_vec())]);
        let source = ManifestSource::new(format!("{base}/releases/manifest.json"));
        let releases = source.get_releases().unwrap();
        let versions: Vec<&str> = releases.iter().map(|r| r.version()).collect();
        assert_eq!(versions, vec!["2.0.0", "1.0.0"]);
        // The relative asset url resolved against the manifest URL's directory (same stub host).
        assert_eq!(
            releases[0].assets()[0].download_url(),
            format!("{base}/releases/app-2.0.0.tar.gz")
        );
    }

    #[test]
    fn get_releases_over_the_stub_propagates_http_error() {
        // A non-2xx manifest response must surface as a structured error, before any parse.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let out = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        let source = ManifestSource::new(format!("{base}/manifest.json"));
        let err = source.get_releases().expect_err("503 must error");
        assert!(
            matches!(err, crate::errors::Error::HttpStatus { status: 503, .. }),
            "a 503 manifest response must surface as HttpStatus, got {err:?}"
        );
    }

    /// Serve one raw HTTP response with an explicit status line and empty body, over a loopback
    /// listener. Used to check that a non-2xx manifest fetch maps to the right structured error.
    fn stub_status(status_line: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let out = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        base
    }

    #[test]
    fn get_releases_over_the_stub_maps_404_to_not_found() {
        let base = stub_status("404 Not Found");
        let source = ManifestSource::new(format!("{base}/manifest.json"));
        let err = source.get_releases().expect_err("404 must error");
        assert!(
            matches!(err, crate::errors::Error::NotFound { .. }),
            "a 404 manifest response must surface as NotFound, got {err:?}"
        );
    }

    #[test]
    fn get_releases_over_the_stub_maps_401_to_unauthorized() {
        let base = stub_status("401 Unauthorized");
        let source = ManifestSource::new(format!("{base}/manifest.json"));
        let err = source.get_releases().expect_err("401 must error");
        assert!(
            matches!(err, crate::errors::Error::Unauthorized { status: 401, .. }),
            "a 401 manifest response must surface as Unauthorized, got {err:?}"
        );
    }

    #[test]
    fn get_releases_over_the_stub_maps_403_to_unauthorized() {
        let base = stub_status("403 Forbidden");
        let source = ManifestSource::new(format!("{base}/manifest.json"));
        let err = source.get_releases().expect_err("403 must error");
        assert!(
            matches!(err, crate::errors::Error::Unauthorized { status: 403, .. }),
            "a 403 manifest response must surface as Unauthorized, got {err:?}"
        );
    }

    #[test]
    fn get_releases_over_the_stub_empty_releases_yields_no_release_found() {
        // An empty (but valid) manifest parses fine, but the latest-release selection has nothing
        // to pick -> Error::NoReleaseFound. This is the update-path surface of an empty manifest.
        let base = stub(vec![(
            "application/json",
            br#"{ "schema": 1, "releases": [] }"#.to_vec(),
        )]);
        let source = ManifestSource::new(format!("{base}/manifest.json"));
        let err = source
            .get_latest_release()
            .expect_err("an empty manifest must have no latest release");
        assert!(
            matches!(err, crate::errors::Error::NoReleaseFound { .. }),
            "an empty releases list must yield NoReleaseFound, got {err:?}"
        );
    }

    #[test]
    fn get_releases_over_the_stub_non_utf8_body_errors() {
        // A manifest body that is not valid UTF-8 must be a structured InvalidResponse (from
        // `read_body`), never a panic. `0xff 0xfe` is not valid UTF-8.
        let base = stub(vec![("application/json", vec![0xff, 0xfe, 0x00, 0x01])]);
        let source = ManifestSource::new(format!("{base}/manifest.json"));
        let err = source
            .get_releases()
            .expect_err("a non-UTF-8 manifest body must error");
        assert!(
            matches!(err, crate::errors::Error::InvalidResponse { .. }),
            "a non-UTF-8 body must be InvalidResponse, got {err:?}"
        );
    }

    /// Serve one JSON response but capture the raw request bytes the client sent, so a test can
    /// assert which headers actually reached the manifest fetch. Returns `(base_url, captured)`.
    fn stub_capturing_request(
        body: Vec<u8>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_thread = std::sync::Arc::clone(&captured);
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                *captured_thread.lock().unwrap() = buf[..n].to_vec();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        (base, captured)
    }

    #[test]
    fn manifest_fetch_sends_accept_json_and_custom_request_header() {
        // Transport-control: the base `Accept: application/json` header and a user-supplied
        // `request_header(..)` must both reach the manifest fetch. A loopback stub captures the
        // raw request line + headers so we can assert the bytes actually went out.
        let (base, captured) =
            stub_capturing_request(br#"{ "schema": 1, "releases": [] }"#.to_vec());
        let mut source = ManifestSource::new(format!("{base}/manifest.json"));
        source.request_header("X-Manifest-Probe", "sentinel-value");
        // Empty manifest -> NoReleaseFound is fine; we only care that the request went out.
        let _ = source.get_releases();

        let raw = captured.lock().unwrap().clone();
        let text = String::from_utf8_lossy(&raw);
        let lower = text.to_ascii_lowercase();
        assert!(
            lower.contains("accept: application/json"),
            "the manifest fetch must send `Accept: application/json`; raw request was:\n{text}"
        );
        assert!(
            text.contains("sentinel-value") && lower.contains("x-manifest-probe:"),
            "a custom request_header(..) must reach the manifest fetch; raw request was:\n{text}"
        );
    }

    /// Build a tiny tar.gz in memory containing a single file named `app` (the default
    /// `bin_path_in_archive` on a unix target, where EXE_SUFFIX is empty).
    #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
    fn app_tar_gz(payload: &[u8]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_path("app").unwrap();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append(&header, payload).unwrap();
        let tar_bytes = tar.into_inner().unwrap();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&tar_bytes).unwrap();
        enc.finish().unwrap()
    }

    #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
    #[test]
    fn update_downloads_and_installs_from_a_manifest() {
        // Full sync flow: the stub serves the manifest (connection 1) then the tar.gz asset
        // (connection 2). The updater compares versions, selects the asset, downloads it, extracts
        // `app`, and installs it to a temp path. A successful `Updated` status plus the installed
        // file proves the whole pipeline ran over the manifest backend.
        let payload = b"installed-binary-payload";
        let archive = app_tar_gz(payload);

        // Bind first so we know the base, then build the manifest referencing a sibling asset URL.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let manifest = r#"{ "schema": 1, "releases": [
            { "version": "2.0.0", "assets": [
                { "name": "app.tar.gz", "url": "app.tar.gz" } ] } ] }"#;
        let manifest_bytes = manifest.as_bytes().to_vec();
        std::thread::spawn(move || {
            let responses: Vec<(&str, Vec<u8>)> = vec![
                ("application/json", manifest_bytes),
                ("application/octet-stream", archive),
            ];
            for (content_type, body) in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    content_type,
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });

        let install_dir = tempfile::tempdir().unwrap();
        let install_path = install_dir.path().join("installed-app");

        let status = Update::configure()
            .manifest_url(format!("{base}/releases/manifest.json"))
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .bin_install_path(&install_path)
            .no_confirm(true)
            .show_output(false)
            // Pick the single served asset directly, sidestepping target-name matching.
            .asset_matcher(|assets| assets.first().cloned())
            .build()
            .unwrap()
            .update_extended()
            .expect("the update must download and install from the manifest");

        assert!(
            status.is_updated(),
            "a newer manifest release served as a real tar.gz must install -> Updated, got {status:?}"
        );
        assert_eq!(status.version(), Some("2.0.0"));
        assert!(
            install_path.exists(),
            "the pipeline must install the extracted binary to {install_path:?}"
        );
        assert_eq!(std::fs::read(&install_path).unwrap(), payload);
    }

    #[cfg(feature = "async")]
    mod async_tests {
        use super::super::{ManifestSource, Update};
        use crate::update::AsyncReleaseSource;
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        fn stub(responses: Vec<(&'static str, Vec<u8>)>) -> String {
            super::stub(responses)
        }

        #[tokio::test]
        async fn get_releases_async_over_the_stub_parses_the_manifest() {
            let manifest = r#"{ "schema": 1, "releases": [
                { "version": "2.0.0", "assets": [
                    { "name": "app-2.0.0.tar.gz", "url": "app-2.0.0.tar.gz" } ] } ] }"#;
            let base = stub(vec![("application/json", manifest.as_bytes().to_vec())]);
            let source = ManifestSource::new(format!("{base}/releases/manifest.json"));
            let releases = source.get_releases().await.unwrap();
            assert_eq!(releases.len(), 1);
            assert_eq!(releases[0].version(), "2.0.0");
            assert_eq!(
                releases[0].assets()[0].download_url(),
                format!("{base}/releases/app-2.0.0.tar.gz")
            );
        }

        #[tokio::test]
        async fn get_releases_async_empty_releases_yields_no_release_found() {
            // Async parity: an empty manifest over the async transport has no latest release.
            let base = super::stub(vec![(
                "application/json",
                br#"{ "schema": 1, "releases": [] }"#.to_vec(),
            )]);
            let source = ManifestSource::new(format!("{base}/manifest.json"));
            let err = source
                .get_latest_release()
                .await
                .expect_err("an empty manifest must have no latest release");
            assert!(
                matches!(err, crate::errors::Error::NoReleaseFound { .. }),
                "async empty releases must yield NoReleaseFound, got {err:?}"
            );
        }

        #[tokio::test]
        async fn get_releases_async_maps_404_to_not_found() {
            // Async parity: a non-2xx manifest fetch surfaces as the same structured status error.
            let base = super::stub_status("404 Not Found");
            let source = ManifestSource::new(format!("{base}/manifest.json"));
            let err = source.get_releases().await.expect_err("404 must error");
            assert!(
                matches!(err, crate::errors::Error::NotFound { .. }),
                "async 404 must surface as NotFound, got {err:?}"
            );
        }

        #[tokio::test]
        async fn manifest_fetch_async_sends_accept_json_and_custom_request_header() {
            // Async parity: the base Accept header and a custom request_header both reach the
            // async manifest fetch.
            let (base, captured) =
                super::stub_capturing_request(br#"{ "schema": 1, "releases": [] }"#.to_vec());
            let mut source = ManifestSource::new(format!("{base}/manifest.json"));
            source.request_header("X-Manifest-Probe", "sentinel-value");
            let _ = source.get_releases().await;

            let raw = captured.lock().unwrap().clone();
            let text = String::from_utf8_lossy(&raw);
            let lower = text.to_ascii_lowercase();
            assert!(
                lower.contains("accept: application/json"),
                "async manifest fetch must send Accept: application/json; raw:\n{text}"
            );
            assert!(
                text.contains("sentinel-value") && lower.contains("x-manifest-probe:"),
                "async custom request_header must reach the fetch; raw:\n{text}"
            );
        }

        #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
        #[tokio::test]
        async fn update_async_downloads_and_installs_from_a_manifest() {
            // Async sibling of the sync end-to-end flow: the manifest is fetched over the async
            // transport, then the archive is downloaded and installed through the spawn_blocking
            // finish tail.
            let payload = b"async-installed-binary-payload";
            let archive = super::app_tar_gz(payload);

            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let manifest = r#"{ "schema": 1, "releases": [
                { "version": "2.0.0", "assets": [
                    { "name": "app.tar.gz", "url": "app.tar.gz" } ] } ] }"#;
            let manifest_bytes = manifest.as_bytes().to_vec();
            std::thread::spawn(move || {
                let responses: Vec<(&str, Vec<u8>)> = vec![
                    ("application/json", manifest_bytes),
                    ("application/octet-stream", archive),
                ];
                for (content_type, body) in responses {
                    let (mut stream, _) = match listener.accept() {
                        Ok(c) => c,
                        Err(_) => return,
                    };
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        content_type,
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&body);
                    let _ = stream.flush();
                }
            });

            let install_dir = tempfile::tempdir().unwrap();
            let install_path = install_dir.path().join("installed-app");

            let status = Update::configure()
                .manifest_url(format!("{base}/releases/manifest.json"))
                .bin_name("app")
                .target("x86_64-unknown-linux-gnu")
                .current_version("1.0.0")
                .bin_install_path(&install_path)
                .no_confirm(true)
                .show_output(false)
                .asset_matcher(|assets| assets.first().cloned())
                .build_async()
                .unwrap()
                .update_extended_async()
                .await
                .expect("the async update must download and install from the manifest");

            assert!(
                status.is_updated(),
                "async update must install -> Updated, got {status:?}"
            );
            assert_eq!(status.version(), Some("2.0.0"));
            assert!(install_path.exists());
            assert_eq!(std::fs::read(&install_path).unwrap(), payload);
        }
    }
}

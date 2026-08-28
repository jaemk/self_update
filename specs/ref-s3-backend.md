# S3 backend (reference)

Status: implemented

## Scope

The S3 (and S3-compatible) release backend in `src/backends/s3.rs`. Lists release
artifacts stored as objects in an S3-style bucket, parses the bucket-listing XML
into `Release`/`ReleaseAsset` values, and drives downloads/installs. Targets AWS
S3, AWS S3 dual-stack, Google Cloud Storage (GCS), DigitalOcean Spaces, and any
generic S3-compatible endpoint. Private-bucket request signing (AWS SigV4) is
behind the optional `s3-auth` feature. This file documents existing behavior as a
canonical reference; it does not propose changes.

## Behavior

### Builders

Two builders, each reached through a `configure()` entry point:

- `ReleaseList` / `ReleaseListBuilder` (`src/backends/s3.rs:ReleaseList`,
  `src/backends/s3.rs:ReleaseListBuilder`): queries a bucket
  and returns a `Releases` via `ReleaseList::fetch` (`src/backends/s3.rs:ReleaseList::fetch`) or, under the
  `async` feature, `ReleaseList::fetch_async` (`src/backends/s3.rs:ReleaseList::fetch_async`). The result is a bare listing
  (`current_version()` is `None`); recover the `Vec<Release>` with `into_vec()`.
  `ReleaseList::configure` (`src/backends/s3.rs:ReleaseList::configure`) seeds the builder. Setters: `bucket_name`,
  `asset_prefix`, `asset_key_pattern`, `region`, `endpoint`, `filter_target`, `max_keys`, and
  (under `s3-auth`) `access_key` and `signature_ttl`; plus the shared
  `request_config_setters!(request)`.
  There is **no** `auth_token` setter on this builder (the deprecated no-op was removed);
  the credential setter is `access_key`.
- `Update` / `UpdateBuilder` (`src/backends/s3.rs:Update`, `src/backends/s3.rs:UpdateBuilder`): the `ReleaseUpdate`
  implementation. `Update::configure` returns an `UpdateBuilder`.
  `build` (`src/backends/s3.rs:UpdateBuilder::build`) and `build_async` (under `async`, `src/backends/s3.rs:UpdateBuilder::build_async`) both return
  the concrete `Update` (which is `Send` and exposes the update verbs as inherent
  methods, so no trait import is needed). Backend setters mirror the list builder
  (`endpoint`, `bucket_name`, `asset_prefix`, `asset_key_pattern`, `region`, `access_key`); the common
  setters come from `impl_common_builder_setters!(no_auth_token)` (`src/macros.rs:impl_common_builder_setters`).
  As on the list builder, there is **no** `auth_token` setter (the deprecated shim was
  removed); use `access_key`.

`filter_target` on the list builder drops whole releases that carry no matching
asset (`src/backends/s3.rs:filter_target`, via `has_target_asset` in `fetch`, `src/backends/s3.rs:ReleaseList::fetch`); the `Update`
`target` (a common setter) selects which asset of the chosen release to download.

Both `build` paths require `bucket_name`, bailing `Error::MissingField { field }` with
"`bucket_name` required" otherwise (`src/backends/s3.rs:ReleaseListBuilder::build`, `src/backends/s3.rs:UpdateBuilder::build_update`). They also validate
the endpoint/region pairing up front via `check_endpoint_region` (`src/backends/s3.rs:check_endpoint_region`),
called from `ReleaseListBuilder::build` (`src/backends/s3.rs:ReleaseListBuilder::build`) and
`UpdateBuilder::build_update` (`src/backends/s3.rs:UpdateBuilder::build_update`), so a missing required region is an
`Error::MissingField { field }` from `build()` rather than from the first request. All the string
setters (`bucket_name`, `asset_prefix`, `region`, `filter_target`, and the common
setters) take `impl Into<String>`.

### URL / endpoint composition

`Endpoint` is `#[non_exhaustive]`, derives
`Default` (defaulting to `S3`), and has `From<&str>` / `From<String>` impls that both
produce `Generic(String)` (the variant is now a tuple variant, renamed from
`Generic { end_point }`). The builder setter is `endpoint(impl Into<Endpoint>)` (renamed
from `end_point`). `build_s3_api_url` returns `(download_base_url, api_url)`:

- `S3`: `https://<bucket>.s3.<region>.amazonaws.com/` (`src/backends/s3.rs:build_s3_api_url`)
- `S3DualStack`: `https://<bucket>.s3.dualstack.<region>.amazonaws.com/` (`src/backends/s3.rs:build_s3_api_url`)
- `DigitalOceanSpaces`: `https://<bucket>.<region>.digitaloceanspaces.com/` (`src/backends/s3.rs:build_s3_api_url`)
- `GCS`: `https://storage.googleapis.com/<bucket>/` (region not consumed) (`src/backends/s3.rs:build_s3_api_url`)
- `Generic(endpoint)`: the supplied URL used verbatim as the base

`region` is `Option<String>`. The three host-interpolating endpoints (`S3`,
`S3DualStack`, `DigitalOceanSpaces`) require it (`endpoint_requires_region`,
`src/backends/s3.rs:endpoint_requires_region`): a missing region surfaces as `Error::MissingField { field }` (field `region`,
for the S3, S3DualStack, and DigitalOceanSpaces endpoints). This is
now validated at `build()` time via `check_endpoint_region` (`src/backends/s3.rs:check_endpoint_region`), not
deferred to URL construction. `GCS` and `Generic` never read the region and build
without it (under `s3-auth`, SigV4 still defaults the signing region to `us-east-1`
when none is set).

### Listing + max_keys + continuation + prefix

`max_keys` is a `u16` field on the `Update` / `ReleaseList` builders, defaulting to 1000 (the
ListObjectsV2 cap). The `max_keys(u16)` setter clamps to `1..=1000` via
`clamp_max_keys`. The listing query string is appended to the download base:

- S3 / S3DualStack / DigitalOceanSpaces / Generic:
  `?list-type=2&max-keys=<max_keys><prefix><continuation>` (the ListBucket v2 API)
- GCS: `?max-keys=<max_keys><prefix><continuation>` (no `list-type=2`, which is S3-specific)

`asset_prefix`, when set, is appended as `&prefix=<value>`; when `None` the segment is absent.

The listing is described transport-free as a `PageRequest<Release>` (`s3_listing_plan` ->
`s3_page`) and driven by the sans-io `run_paginated` / `run_paginated_async` drivers. The parser
reads `<IsTruncated>true</IsTruncated>` and `<NextContinuationToken>`, and when truncated emits
`Page::next` as a fresh `PageRequest` with `&continuation-token=<token>` in the query, which the
same driver follows. So a >1000-key bucket is walked across multiple requests, not truncated. Under
`s3-auth` each continuation URL is freshly SigV4-signed. The `signature_ttl(Duration)` setter
(default 300s, clamped to AWS's `X-Amz-Expires` range of 1s..=7d via
`clamp_signature_ttl`, `src/backends/s3.rs:clamp_signature_ttl`) sets the `X-Amz-Expires` of signed listing and
download URLs.

### XML to model

`fetch_releases_from_s3` (`src/backends/s3.rs:fetch_releases`) builds the URL, sends one GET via `send`
(`src/backends/mod.rs:send`), reads the body as text, and hands it to `parse_s3_response`
(`src/backends/s3.rs:parse_s3_response`). Parsing uses `quick_xml::Reader` with `trim_text(true)` (`src/backends/s3.rs:parse_s3_response`)
and walks the `ListBucketResult`:

- A `<Contents>` start flushes any in-progress release via `add_to_releases_list`
  and resets state (`src/backends/s3.rs:parse_s3_response`).
- `<Key>` text is matched against the filename regex (below); on match it sets the
  current release's `name`/`version`, forms the download URL as
  `download_base_url + key` (`src/backends/s3.rs:parse_s3_response`), and sets a single-element `assets` vec
  whose `name` is the key's filename component (path stripped via
  `PathBuf::file_name`, `src/backends/s3.rs:parse_s3_response`). A non-matching key is logged and skipped
  (`src/backends/s3.rs:parse_s3_response`).
- `<LastModified>` text sets the release `date` (`src/backends/s3.rs:parse_s3_response`).
- `Eof` flushes the final in-progress release (`src/backends/s3.rs:parse_s3_response`).

`add_to_releases_list` (`src/backends/s3.rs:add_to_releases_list`) drops any release with an empty `name` or
`version`, and merges entries sharing the same `name`+`version` into one release
with their assets concatenated (`src/backends/s3.rs:add_to_releases_list`); otherwise it pushes a new release.

### Version derivation

By default a single case-insensitive regex parses object keys (`ASSET_KEY_REGEX`):
`(?i)(?P<prefix>.*/)*(?P<name>.+)-[v]{0,1}(?P<version>\d+\.\d+\.\d+)-.+`.
The key must contain a `name-[v]<major>.<minor>.<patch>-<suffix>` shape: `name`
becomes the release name and the dotted triple becomes the version, with any
leading `v` stripped. Keys lacking this shape produce no release. The default
version group captures only the `major.minor.patch` triple, so a pre-release key
like `mybin-0.1.2-beta-x86_64-...` parses lossily as `0.1.2` (#61).

`asset_key_pattern(impl Into<String>)` on both builders replaces the default
matcher with a user-supplied regex tuned to the bucket's key layout (e.g. one
whose `version` group admits a pre-release segment, so `0.1.2-beta` /
`0.1.2-beta.1` round-trip). The pattern must define `name` and `version` named
capture groups; `compile_asset_key_pattern` compiles and validates it at
`build()`, surfacing a pattern that does not compile or lacks a required group as
`Error::InvalidAssetKeyPattern` (a `#[non_exhaustive]` variant gated on the `s3`
feature, `Display` prefix "ConfigError:", underlying error chained via
`source()`). At parse time a custom pattern's captured version (after the same
leading-`v` trim) must parse as semver or the key is skipped like a non-matching
key; the default pattern is exempt from that check since its version group only
matches a numeric triple. When unset, behavior is unchanged.

`ReleaseUpdate` selection helpers operate on the parsed list: `pick_latest`
(`src/backends/s3.rs:pick_latest`) picks the highest version (ignoring unparseable ones, erroring
`Error::NoReleaseFound` when empty); `sort_newer` (`src/backends/s3.rs:sort_newer`) filters to strictly
newer-than-current, newest-first; `find_version` (`src/backends/s3.rs:find_version`) matches an exact
version, erroring `Error::NoReleaseFound` when absent. These back
`get_latest_release`, `get_newer_releases`, and `get_release_version`
(`src/backends/s3.rs:get_latest_release`, `src/backends/s3.rs:get_newer_releases`,
`src/backends/s3.rs:get_release_version`) and their `async` siblings, all also exposed as inherent methods on
`Update` alongside `is_update_available`.

### Signing under s3-auth

The `auth` module (`src/backends/s3.rs:auth`) is gated on `feature = "s3-auth"`. `AccessKey`
(`src/backends/s3.rs:AccessKey`) is `#[non_exhaustive]` with fields `access_key_id` and
`secret_access_key`, built through `AccessKey::new(access_key_id,
secret_access_key)` (`src/backends/s3.rs:AccessKey::new`, both args `impl Into<String>`) or the
`From<(&str, &str)>` / `From<(String, String)>` impls; it is re-exported as
`self_update::backends::s3::AccessKey` (`src/backends/s3.rs:AccessKey`). The `#[non_exhaustive]`
attribute reserves room for a future STS session token; no `session_token` field
exists today.

`s3_signature_v4` (`src/backends/s3.rs:s3_signature_v4`) implements AWS SigV4 presigned-query signing. With
no `AccessKey` it returns the URL unchanged (`src/backends/s3.rs:s3_signature_v4_at`) -- so public buckets are
unsigned. With one it appends `X-Amz-Algorithm=AWS4-HMAC-SHA256`,
`X-Amz-Credential`, `X-Amz-Date`, `X-Amz-Expires`, `X-Amz-SignedHeaders=host`, and
the `X-Amz-Signature` (lowercase hex HMAC-SHA256). Region defaults to `us-east-1`
when absent (`src/backends/s3.rs:s3_signature_v4_at`); the service is fixed to `s3` (`src/backends/s3.rs:derive_signing_key`) and the
payload to `UNSIGNED-PAYLOAD` (`src/backends/s3.rs:s3_signature_v4_at`). Signing uses `hmac`/`sha2` for
HMAC-SHA256 and SHA-256, `percent-encoding` for URI encoding (reserving
`- . _ ~`, slash kept in the canonical URI but encoded in query params,
`src/backends/s3.rs:uri_encode`), `url` for parsing, and `time` for the timestamp. Both the listing URL
(TTL 300s, `src/backends/s3.rs:build_s3_api_url`) and each asset download URL (TTL 300s, `src/backends/s3.rs:parse_s3_response`) are
signed when an access key is present.

The s3 backend does not authenticate via bearer token. The shared `auth_token`
setter is omitted via `impl_common_builder_setters!(no_auth_token)`; there is no
`auth_token` method at all on the s3 builders (use `.access_key((id, secret))`
under `s3-auth`). The shared auth derivation is a no-op for s3 (no `Authorization`,
no `User-Agent`): `api_headers` uses the `UpdateConfig` trait default, which is a
no-op, because s3 authenticates by SigV4-signing the URL, not via an auth header.

### Errors

A non-2xx listing response is always an `Err`, never an `Ok` parsed from the error
body: `send` / `http_client::get` bail on any non-2xx status before returning
(`src/backends/mod.rs:send`, `src/http_client/mod.rs:HttpClient::get`). Both clients now map a completed non-2xx to the same structured
variant by status: 404 -> `Error::NotFound`, 401/403 -> `Error::Unauthorized`,
any other non-2xx -> `Error::HttpStatus` (`status_to_error`, `src/errors.rs:status_to_error`); a
request that cannot complete (connection/TLS/timeout) is `Error::Transport`. XML
parse errors surface as `Error::InvalidResponse` with the buffer position (`src/backends/s3.rs:parse_s3_response`).
Missing region (for the region-requiring endpoints) and missing bucket are both
`Error::MissingField { field }`, now raised from `build()` rather than the first request.

## Public surface

- `s3::Endpoint` (`#[non_exhaustive]`, `Default = S3`) with variants `S3`,
  `S3DualStack`, `GCS`, `DigitalOceanSpaces`, `Generic(String)`; plus
  `From<&str>` / `From<String>` -> `Generic`.
- `s3::ReleaseList`, `s3::ReleaseListBuilder` (setters: `bucket_name`,
  `asset_prefix`, `asset_key_pattern`, `region`, `endpoint`, `filter_target`,
  `max_keys`, `access_key` / `signature_ttl` [s3-auth], request-config setters,
  `build`); `ReleaseList::fetch` and `fetch_async` [async].
- `s3::UpdateBuilder`, `s3::Update` (`#[non_exhaustive]`); `Update::configure`,
  `build` -> `Update`, `build_async` -> `Update` [async]. `Update` is `Send` with
  the inherent verbs (`update`, `update_extended`, `get_latest_release`,
  `get_newer_releases`, `get_release_version`, `is_update_available`).
- `s3::AccessKey` [s3-auth], re-exported, `#[non_exhaustive]`, `AccessKey::new`
  plus tuple `From` impls.

## Invariants and regression checklist

- `bucket_name` required on both builders -> `Error::MissingField { field }`.
- Region required for `S3`/`S3DualStack`/`DigitalOceanSpaces`; ignored for
  `GCS`/`Generic`. Missing required region -> `Error::MissingField { field }`.
- S3-family and Generic listing query uses `list-type=2&max-keys=<max_keys>` (default 1000); GCS
  uses `max-keys=<max_keys>` only (no `list-type=2`). `max_keys` clamps to `1..=1000`.
- A truncated listing (`<IsTruncated>true</IsTruncated>` + `<NextContinuationToken>`) is followed
  via `&continuation-token=<token>`, so a >1000-key bucket is walked across requests. Under
  `s3-auth` each continuation URL is freshly signed; `signature_ttl` sets the `X-Amz-Expires`.
- `asset_prefix` appended as `&prefix=<value>`; absent when unset.
- Asset `name` is the key's filename component, not the full key path.
- Default version regex requires a `\d+\.\d+\.\d+` triple; leading `v` stripped;
  keys not matching produce no release. A pre-release key parses lossily as the
  bare triple (`0.1.2-beta` -> `0.1.2`); that default is pinned.
- `asset_key_pattern` replaces the default matcher on both builders; it must
  define `name` and `version` named groups, is validated at `build()`
  (`Error::InvalidAssetKeyPattern`, source chained), and a custom capture that is
  not semver (after the leading-`v` trim) skips the key.
- Releases with the same `name`+`version` merge their assets; empty name/version
  dropped.
- Non-2xx listing response is an `Err`, never `Ok` from the error body.
- Public buckets (no access key) emit unsigned URLs; with `s3-auth` + access key,
  both listing and asset URLs are SigV4-signed (TTL 300s), region defaulting to
  `us-east-1`.
- the `auth_token` setter was removed from the s3 builders; use `access_key((id, secret))` under `s3-auth` to authenticate.
- `Endpoint`, `Update`, and `AccessKey` are `#[non_exhaustive]`.
- `max_keys` takes a `u16`; `signature_ttl` clamps to 1s..=7d.

## Tests

In-module tests (`src/backends/s3.rs:tests`): `parse_s3_response` cases (single/multi asset,
v-prefix strip, multiple releases, non-matching-key skip, path-stripped filename,
malformed-XML error, empty body); custom `asset_key_pattern` cases (pre-release
kept and merged, non-semver capture skipped, default lossy pre-release pinned,
bad-pattern / missing-group `build()` errors on both builders, end-to-end
threading through `Update::get_latest_release` and `ReleaseList::fetch` over the
loopback stub); `add_to_releases_list` empty-name/version drop;
loopback-TCP stub tests for the sync and async `ReleaseUpdate` fetch methods
(`get_latest_release`, `get_newer_releases`, `get_release_version`,
`is_update_available`, multi-asset merge); the non-2xx error contract
(`assert_non_2xx_err`); `pick_latest`/`sort_newer`/`find_version` unit tests;
`build_s3_api_url` shape tests per endpoint (S3, dual-stack, DigitalOcean, GCS,
Generic), prefix append, and missing-region error; and (under `s3-auth`)
`s3_signature_v4` structural invariants, region default, listing-URL signing, and
asset-URL signing, plus `AccessKey` re-export/tuple-`From` coverage.

## Related

- `s3-auth-token-removal.md`
- `s3-max-keys-configurable.md`
- `transport-control.md`
- `error-network-vs-http-semantics.md`
- `async-api.md`

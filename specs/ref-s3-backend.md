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

- `ReleaseList` / `ReleaseListBuilder` (`s3.rs:91`, `s3.rs:151`): queries a bucket
  and returns a `Releases` via `ReleaseList::fetch` (`s3.rs:220`). The result is a bare listing
  (`current_version()` is `None`); recover the `Vec<Release>` with `into_vec()`.
  `ReleaseList::configure` (`s3.rs:205`) seeds the builder. Setters: `bucket_name`,
  `asset_prefix`, `region`, `endpoint`, `filter_target`, `max_keys`, and (under `s3-auth`)
  `access_key` and `signature_ttl`; plus the shared `request_config_setters!(request)`.
  There is **no** `auth_token` setter on this builder (the deprecated no-op was removed);
  the credential setter is `access_key`.
- `Update` / `UpdateBuilder` (`s3.rs:359`, `s3.rs:247`): the `ReleaseUpdate`
  implementation. `Update::configure` (`s3.rs:371`) returns an `UpdateBuilder`.
  `build` returns `Box<dyn ReleaseUpdate>` (`s3.rs:337`); `build_async` (under
  `async`) returns the concrete `Update` so the inherent `*_async` methods are
  reachable (`s3.rs:346`). Backend setters mirror the list builder
  (`endpoint`, `bucket_name`, `asset_prefix`, `region`, `access_key`); the common
  setters come from `impl_common_builder_setters!(no_auth_token)` (`s3.rs:314`).
  As on the list builder, there is **no** `auth_token` setter (the deprecated shim was
  removed); use `access_key`.

`filter_target` on the list builder drops whole releases that carry no matching
asset (`s3.rs:144`, via `has_target_asset` in `fetch`, `s3.rs:234`); the `Update`
`target` (a common setter) selects which asset of the chosen release to download.

Both `build` paths require `bucket_name`, bailing `Error::MissingField { field }` with
"`bucket_name` required" otherwise (`s3.rs:177`, `s3.rs:323`). They also validate
the endpoint/region pairing up front via `check_endpoint_region` (`s3.rs:78`),
called from `ReleaseListBuilder::build` (`s3.rs:171`) and
`UpdateBuilder::build_update` (`s3.rs:317`), so a missing required region is an
`Error::MissingField { field }` from `build()` rather than from the first request. All the string
setters (`bucket_name`, `asset_prefix`, `region`, `filter_target`, and the common
setters) take `impl Into<String>`.

### URL / endpoint composition

`Endpoint` is `#[non_exhaustive]`, derives
`Default` (defaulting to `S3`), and has `From<&str>` / `From<String>` impls that both
produce `Generic(String)` (the variant is now a tuple variant, renamed from
`Generic { end_point }`). The builder setter is `endpoint(impl Into<Endpoint>)` (renamed
from `end_point`). `build_s3_api_url` returns `(download_base_url, api_url)`:

- `S3`: `https://<bucket>.s3.<region>.amazonaws.com/` (`s3.rs:706`)
- `S3DualStack`: `https://<bucket>.s3.dualstack.<region>.amazonaws.com/` (`s3.rs:710`)
- `DigitalOceanSpaces`: `https://<bucket>.<region>.digitaloceanspaces.com/` (`s3.rs:714`)
- `GCS`: `https://storage.googleapis.com/<bucket>/` (region not consumed) (`s3.rs:718`)
- `Generic(endpoint)`: the supplied URL used verbatim as the base

`region` is `Option<String>`. The three host-interpolating endpoints (`S3`,
`S3DualStack`, `DigitalOceanSpaces`) require it (`endpoint_requires_region`,
`s3.rs:69`): a missing region surfaces as `Error::MissingField { field }` (field `region`,
for the S3, S3DualStack, and DigitalOceanSpaces endpoints). This is
now validated at `build()` time via `check_endpoint_region` (`s3.rs:78`), not
deferred to URL construction. `GCS` and `Generic` never read the region and build
without it (under `s3-auth`, SigV4 still defaults the signing region to `us-east-1`
when none is set).

### Listing + max_keys + continuation + prefix

`max_keys` is a `u16` field on the `Update` / `ReleaseList` builders, defaulting to 1000 (the
ListObjectsV2 cap). The `max_keys(impl Into<u16>)` setter clamps to `1..=1000` via
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
(default 300s) sets the `X-Amz-Expires` of signed listing and download URLs.

### XML to model

`fetch_releases_from_s3` (`s3.rs:743`) builds the URL, sends one GET via `send`
(`s3.rs:763`), reads the body as text, and hands it to `parse_s3_response`
(`s3.rs:814`). Parsing uses `quick_xml::Reader` with `trim_text(true)` (`s3.rs:821`)
and walks the `ListBucketResult`:

- A `<Contents>` start flushes any in-progress release via `add_to_releases_list`
  and resets state (`s3.rs:848`).
- `<Key>` text is matched against the filename regex (below); on match it sets the
  current release's `name`/`version`, forms the download URL as
  `download_base_url + key` (`s3.rs:875`), and sets a single-element `assets` vec
  whose `name` is the key's filename component (path stripped via
  `PathBuf::file_name`, `s3.rs:865`). A non-matching key is logged and skipped
  (`s3.rs:887`).
- `<LastModified>` text sets the release `date` (`s3.rs:890`).
- `Eof` flushes the final in-progress release (`s3.rs:898`).

`add_to_releases_list` (`s3.rs:921`) drops any release with an empty `name` or
`version`, and merges entries sharing the same `name`+`version` into one release
with their assets concatenated (`s3.rs:923`); otherwise it pushes a new release.

### Version derivation

A single case-insensitive regex parses object keys (`s3.rs:834`):
`(?i)(?P<prefix>.*/)*(?P<name>.+)-[v]{0,1}(?P<version>\d+\.\d+\.\d+)-.+`.
The key must contain a `name-[v]<major>.<minor>.<patch>-<suffix>` shape: `name`
becomes the release name and the dotted triple becomes the version, with any
leading `v` stripped (`s3.rs:874`). Keys lacking this shape produce no release.
Regex construction failure surfaces as `Error::InvalidResponse` (`s3.rs:836`).

`ReleaseUpdate` selection helpers operate on the parsed list: `pick_latest`
(`s3.rs:406`) picks the highest version (ignoring unparseable ones, erroring
"No release was found" when empty); `sort_newer` (`s3.rs:428`) filters to strictly
newer-than-current, newest-first; `find_version` (`s3.rs:449`) matches an exact
version, erroring `Error::Release` when absent. These back `get_latest_release`,
`get_latest_releases`, and `get_release_version` (`s3.rs:475`) and their `async`
siblings (`s3.rs:484`).

### Signing under s3-auth

The `auth` module (`s3.rs:503`) is gated on `feature = "s3-auth"`. `AccessKey`
(`s3.rs:524`) is `#[non_exhaustive]` with fields `access_key_id` and
`secret_access_key`, built through `AccessKey::new(access_key_id,
secret_access_key)` (`s3.rs:533`, both args `impl Into<String>`) or the
`From<(&str, &str)>` / `From<(String, String)>` impls; it is re-exported as
`self_update::backends::s3::AccessKey` (`s3.rs:26`). The `#[non_exhaustive]`
attribute reserves room for a future STS session token; no `session_token` field
exists today.

`s3_signature_v4` (`s3.rs:612`) implements AWS SigV4 presigned-query signing. With
no `AccessKey` it returns the URL unchanged (`s3.rs:620`) -- so public buckets are
unsigned. With one it appends `X-Amz-Algorithm=AWS4-HMAC-SHA256`,
`X-Amz-Credential`, `X-Amz-Date`, `X-Amz-Expires`, `X-Amz-SignedHeaders=host`, and
the `X-Amz-Signature` (lowercase hex HMAC-SHA256). Region defaults to `us-east-1`
when absent (`s3.rs:635`); the service is fixed to `s3` (`s3.rs:596`) and the
payload to `UNSIGNED-PAYLOAD` (`s3.rs:667`). Signing uses `hmac`/`sha2` for
HMAC-SHA256 and SHA-256, `percent-encoding` for URI encoding (reserving
`- . _ ~`, slash kept in the canonical URI but encoded in query params,
`s3.rs:561`), `url` for parsing, and `time` for the timestamp. Both the listing URL
(TTL 300s, `s3.rs:734`) and each asset download URL (TTL 300s, `s3.rs:879`) are
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
(`s3.rs:763`). Both clients now map a completed non-2xx to the same structured
variant by status: 404 -> `Error::NotFound`, 401/403 -> `Error::Unauthorized`,
any other non-2xx -> `Error::HttpStatus` (`status_to_error`, `errors.rs:254`); a
request that cannot complete (connection/TLS/timeout) is `Error::Transport`. XML
parse errors surface as `Error::InvalidResponse` with the buffer position (`s3.rs:904`).
Missing region (for the region-requiring endpoints) and missing bucket are both
`Error::MissingField { field }`, now raised from `build()` rather than the first request.

## Public surface

- `s3::Endpoint` (`#[non_exhaustive]`, `Default = S3`) with variants `S3`,
  `S3DualStack`, `GCS`, `DigitalOceanSpaces`, `Generic(String)`; plus
  `From<&str>` / `From<String>` -> `Generic`.
- `s3::ReleaseList`, `s3::ReleaseListBuilder` (setters: `bucket_name`,
  `asset_prefix`, `region`, `endpoint`, `filter_target`, `access_key` [s3-auth],
  request-config setters, `build`).
- `s3::UpdateBuilder`, `s3::Update` (`#[non_exhaustive]`); `Update::configure`,
  `build` -> `Box<dyn ReleaseUpdate>`, `build_async` -> `Update` [async].
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
- Version regex requires a `\d+\.\d+\.\d+` triple; leading `v` stripped; keys not
  matching produce no release.
- Releases with the same `name`+`version` merge their assets; empty name/version
  dropped.
- Non-2xx listing response is an `Err`, never `Ok` from the error body.
- Public buckets (no access key) emit unsigned URLs; with `s3-auth` + access key,
  both listing and asset URLs are SigV4-signed (TTL 300s), region defaulting to
  `us-east-1`.
- the `auth_token` setter was removed from the s3 builders; use `access_key((id, secret))` under `s3-auth` to authenticate.
- `EndPoint`, `Update`, and `AccessKey` are `#[non_exhaustive]`.

## Tests

In-module tests (`s3.rs:938`): `parse_s3_response` cases (single/multi asset,
v-prefix strip, multiple releases, non-matching-key skip, path-stripped filename,
malformed-XML error, empty body); `add_to_releases_list` empty-name/version drop;
loopback-TCP stub tests for the sync and async `ReleaseUpdate` fetch methods
(`get_latest_release`, `get_latest_releases`, `get_release_version`,
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

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

- `ReleaseList` / `ReleaseListBuilder` (`s3.rs:67`, `s3.rs:150`): queries a bucket
  and returns a `Vec<Release>` via `ReleaseList::fetch` (`s3.rs:181`).
  `ReleaseList::configure` (`s3.rs:166`) seeds the builder. Setters: `bucket_name`,
  `asset_prefix`, `region`, `end_point`, `filter_target`, and (under `s3-auth`)
  `access_key`; plus the shared `request_config_setters!(request)` (`s3.rs:128`).
- `Update` / `UpdateBuilder` (`s3.rs:202`, `s3.rs:298`): the `ReleaseUpdate`
  implementation. `Update::configure` (`s3.rs:313`) returns an `UpdateBuilder`.
  `build` returns `Box<dyn ReleaseUpdate>` (`s3.rs:279`); `build_async` (under
  `async`) returns the concrete `Update` so the inherent `*_async` methods are
  reachable (`s3.rs:288`). Backend setters mirror the list builder
  (`end_point`, `bucket_name`, `asset_prefix`, `region`, `access_key`); the common
  setters come from `impl_common_builder_setters!(no_auth_token)` (`s3.rs:257`).

`filter_target` on the list builder drops whole releases that carry no matching
asset (`s3.rs:115`, via `has_target_asset` in `fetch`, `s3.rs:191`); the `Update`
`target` (a common setter) selects which asset of the chosen release to download.

Both `build` paths require `bucket_name`, bailing `Error::Config` with
"`bucket_name` required" otherwise (`s3.rs:138`, `s3.rs:265`). They also validate
the endpoint/region pairing up front via `check_endpoint_region` (`s3.rs:78`),
called from `ReleaseListBuilder::build` (`s3.rs:164`) and
`UpdateBuilder::build_update` (`s3.rs:301`), so a missing required region is an
`Error::Config` from `build()` rather than from the first request. All the string
setters (`bucket_name`, `asset_prefix`, `region`, `filter_target`, and the common
setters) take `impl Into<String>`.

### URL / endpoint composition

`EndPoint` (`s3.rs:28`) is `#[non_exhaustive]`, derives `Default` (defaulting to
`S3`, `s3.rs:35`), and has `From<&str>` / `From<String>` impls that both produce
`Generic` (`s3.rs:53`, `s3.rs:61`). `build_s3_api_url` (`s3.rs:619`) returns
`(download_base_url, api_url)`:

- `S3`: `https://<bucket>.s3.<region>.amazonaws.com/` (`s3.rs:636`)
- `S3DualStack`: `https://<bucket>.s3.dualstack.<region>.amazonaws.com/` (`s3.rs:640`)
- `DigitalOceanSpaces`: `https://<bucket>.<region>.digitaloceanspaces.com/` (`s3.rs:644`)
- `GCS`: `https://storage.googleapis.com/<bucket>/` (region not consumed) (`s3.rs:648`)
- `Generic { end_point }`: the supplied URL used verbatim as the base (`s3.rs:649`)

`region` is `Option<String>`. The three host-interpolating endpoints (`S3`,
`S3DualStack`, `DigitalOceanSpaces`) require it (`endpoint_requires_region`,
`s3.rs:69`): a missing region surfaces as `Error::Config("`region` required for the
S3, S3DualStack, and DigitalOceanSpaces endpoints; call `.region(...)`")`. This is
now validated at `build()` time via `check_endpoint_region` (`s3.rs:78`), not
deferred to URL construction. `GCS` and `Generic` never read the region and build
without it (under `s3-auth`, SigV4 still defaults the signing region to `us-east-1`
when none is set).

### Listing + MAX_KEYS + prefix

`MAX_KEYS` is a `const u8 = 100` (`s3.rs:19`), the per-request item cap sent to the
listing API. The listing query string is appended to the download base:

- S3 / S3DualStack / DigitalOceanSpaces / Generic:
  `?list-type=2&max-keys=<MAX_KEYS><prefix>` (the ListBucket v2 API) (`s3.rs:652`)
- GCS: `?max-keys=<MAX_KEYS><prefix>` (no `list-type=2`, which is S3-specific)
  (`s3.rs:660`)

`asset_prefix`, when set, is appended as `&prefix=<value>` (`s3.rs:626`); when
`None` the segment is absent. The listing is a single request: there is no
continuation-token pagination, so at most `MAX_KEYS` objects are listed.

### XML to model

`fetch_releases_from_s3` (`s3.rs:673`) builds the URL, sends one GET via `send`
(`s3.rs:693`), reads the body as text, and hands it to `parse_s3_response`
(`s3.rs:744`). Parsing uses `quick_xml::Reader` with `trim_text(true)` (`s3.rs:751`)
and walks the `ListBucketResult`:

- A `<Contents>` start flushes any in-progress release via `add_to_releases_list`
  and resets state (`s3.rs:778`).
- `<Key>` text is matched against the filename regex (below); on match it sets the
  current release's `name`/`version`, forms the download URL as
  `download_base_url + key` (`s3.rs:805`), and sets a single-element `assets` vec
  whose `name` is the key's filename component (path stripped via
  `PathBuf::file_name`, `s3.rs:794`). A non-matching key is logged and skipped
  (`s3.rs:816`).
- `<LastModified>` text sets the release `date` (`s3.rs:820`).
- `Eof` flushes the final in-progress release (`s3.rs:828`).

`add_to_releases_list` (`s3.rs:851`) drops any release with an empty `name` or
`version`, and merges entries sharing the same `name`+`version` into one release
with their assets concatenated (`s3.rs:857`); otherwise it pushes a new release.

### Version derivation

A single case-insensitive regex parses object keys (`s3.rs:763`):
`(?i)(?P<prefix>.*/)*(?P<name>.+)-[v]{0,1}(?P<version>\d+\.\d+\.\d+)-.+`.
The key must contain a `name-[v]<major>.<minor>.<patch>-<suffix>` shape: `name`
becomes the release name and the dotted triple becomes the version, with any
leading `v` stripped (`s3.rs:804`). Keys lacking this shape produce no release.
Regex construction failure surfaces as `Error::Release` (`s3.rs:765`).

`ReleaseUpdate` selection helpers operate on the parsed list: `pick_latest`
(`s3.rs:348`) picks the highest version (ignoring unparseable ones, erroring
"No release was found" when empty); `sort_newer` (`s3.rs:370`) filters to strictly
newer-than-current, newest-first; `find_version` (`s3.rs:391`) matches an exact
version, erroring `Error::Release` when absent. These back `get_latest_release`,
`get_latest_releases`, and `get_release_version` (`s3.rs:404`) and their `async`
siblings (`s3.rs:424`).

### Signing under s3-auth

The `auth` module (`s3.rs:444`) is gated on `feature = "s3-auth"`. `AccessKey`
(`s3.rs:508`) is `#[non_exhaustive]` with fields `access_key_id` and
`secret_access_key`, built through `AccessKey::new(access_key_id,
secret_access_key)` (`s3.rs:517`, both args `impl Into<String>`) or the
`From<(&str, &str)>` / `From<(String, String)>` impls; it is re-exported as
`self_update::backends::s3::AccessKey` (`s3.rs:26`). The `#[non_exhaustive]`
attribute reserves room for a future STS session token; no `session_token` field
exists today.

`s3_signature_v4` (`s3.rs:542`) implements AWS SigV4 presigned-query signing. With
no `AccessKey` it returns the URL unchanged (`s3.rs:550`) -- so public buckets are
unsigned. With one it appends `X-Amz-Algorithm=AWS4-HMAC-SHA256`,
`X-Amz-Credential`, `X-Amz-Date`, `X-Amz-Expires`, `X-Amz-SignedHeaders=host`, and
the `X-Amz-Signature` (lowercase hex HMAC-SHA256). Region defaults to `us-east-1`
when absent (`s3.rs:565`); the service is fixed to `s3` and the payload to
`UNSIGNED-PAYLOAD` (`s3.rs:597`). Signing uses `hmac`/`sha2` for HMAC-SHA256 and
SHA-256, `percent-encoding` for URI encoding (reserving `- . _ ~`, slash kept in
the canonical URI but encoded in query params, `s3.rs:491`), `url` for parsing, and
`time` for the timestamp. Both the listing URL (TTL 300s, `s3.rs:664`) and each
asset download URL (TTL 300s, `s3.rs:809`) are signed when an access key is
present.

The s3 backend does not authenticate via bearer token. The shared `auth_token`
setter is omitted via `impl_common_builder_setters!(no_auth_token)` (`s3.rs:257`,
macro at `src/macros.rs:228`); the previous no-op `auth_token` setter is removed,
not deprecated. `api_headers` uses the `UpdateConfig` trait default (no override),
so it sets a single `Authorization: token ...` header and no `User-Agent`.

### Errors

A non-2xx listing response is always an `Err`, never an `Ok` parsed from the error
body: `send` / `http_client::get` bail on any non-2xx status before returning
(`s3.rs:692`). Both clients now map a completed non-2xx to the same structured
variant by status: 404 -> `Error::NotFound`, 401/403 -> `Error::Unauthorized`,
any other non-2xx -> `Error::HttpStatus` (`status_to_error`, `errors.rs:254`); a
request that cannot complete (connection/TLS/timeout) is `Error::Transport`. XML
parse errors surface as `Error::Release` with the buffer position (`s3.rs:834`).
Missing region (for the region-requiring endpoints) and missing bucket are both
`Error::Config`, now raised from `build()` rather than the first request.

## Public surface

- `s3::EndPoint` (`#[non_exhaustive]`, `Default = S3`) with variants `S3`,
  `S3DualStack`, `GCS`, `DigitalOceanSpaces`, `Generic { end_point }`; plus
  `From<&str>` / `From<String>` -> `Generic`.
- `s3::ReleaseList`, `s3::ReleaseListBuilder` (setters: `bucket_name`,
  `asset_prefix`, `region`, `end_point`, `filter_target`, `access_key` [s3-auth],
  request-config setters, `build`).
- `s3::UpdateBuilder`, `s3::Update` (`#[non_exhaustive]`); `Update::configure`,
  `build` -> `Box<dyn ReleaseUpdate>`, `build_async` -> `Update` [async].
- `s3::AccessKey` [s3-auth], re-exported, `#[non_exhaustive]`, `AccessKey::new`
  plus tuple `From` impls.

## Invariants and regression checklist

- `bucket_name` required on both builders -> `Error::Config`.
- Region required for `S3`/`S3DualStack`/`DigitalOceanSpaces`; ignored for
  `GCS`/`Generic`. Missing required region -> `Error::Config`.
- S3-family and Generic listing query uses `list-type=2&max-keys=100`; GCS uses
  `max-keys=100` only (no `list-type=2`).
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
- No `auth_token` setter on the s3 builders; `api_headers` sets no `User-Agent`.
- `EndPoint`, `Update`, and `AccessKey` are `#[non_exhaustive]`.

## Tests

In-module tests (`s3.rs:867`): `parse_s3_response` cases (single/multi asset,
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

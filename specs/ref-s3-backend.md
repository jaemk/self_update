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
  `asset_prefix`, `region`, `endpoint`, `filter_target`, `max_keys`, `allow_insecure_http`,
  and (under `s3-auth`) `access_key` and `signature_ttl`; plus the shared
  `request_config_setters!(request)`. There is **no** `auth_token` setter on this builder
  (the deprecated no-op was removed); the credential setter is `access_key`.
- `Update` / `UpdateBuilder` (`s3.rs:359`, `s3.rs:247`): the `ReleaseUpdate`
  implementation. `Update::configure` (`s3.rs:371`) returns an `UpdateBuilder`.
  `build` returns `Box<dyn ReleaseUpdate>` (`s3.rs:337`); `build_async` (under
  `async`) returns the concrete `Update` so the inherent `*_async` methods are
  reachable (`s3.rs:346`). Backend setters mirror the list builder
  (`endpoint`, `bucket_name`, `asset_prefix`, `region`, `access_key`,
  `allow_insecure_http`); the common setters come from
  `impl_common_builder_setters!(no_auth_token)` (`s3.rs:314`).
  As on the list builder, there is **no** `auth_token` setter (the deprecated shim was
  removed); use `access_key`.

`filter_target` on the list builder drops whole releases that carry no matching
asset (`s3.rs:144`, via `has_target_asset` in `fetch`, `s3.rs:234`); the `Update`
`target` (a common setter) selects which asset of the chosen release to download.

Both `build` paths require `bucket_name`, bailing `Error::MissingField { field: "bucket_name" }`
otherwise. They also validate the endpoint/region pairing up front via
`check_endpoint_region`, called from `ReleaseListBuilder::build` and
`UpdateBuilder::build_update`, so a missing required region is an `Error::MissingField`
from `build()` rather than from the first request. For `Generic` endpoints, both builders
also validate the URL scheme: an `http://` URL is rejected with `Error::Config` unless
`allow_insecure_http(true)` is set (see below). All the string setters take `impl Into<String>`.

### URL / endpoint composition

`Endpoint` is `#[non_exhaustive]`, derives `Default` (defaulting to `S3`), and has
`From<&str>` / `From<String>` impls that both produce `Generic(String)`. These impls
always produce `Generic` regardless of the string value: a string that resembles a
known-variant name (`"s3"`, `"GCS"`) is treated as a full endpoint URL, not routed to
the named variant. Use the enum variants directly when you want a named endpoint.

The builder setter is `endpoint(impl Into<Endpoint>)`. `build_s3_api_url` returns
`(download_base_url, api_url)`:

- `S3`: `https://<bucket>.s3.<region>.amazonaws.com/`
- `S3DualStack`: `https://<bucket>.s3.dualstack.<region>.amazonaws.com/`
- `DigitalOceanSpaces`: `https://<bucket>.<region>.digitaloceanspaces.com/`
- `GCS`: `https://storage.googleapis.com/<bucket>/` (region not consumed)
- `Generic(endpoint)`: the supplied URL used verbatim as the base

`region` is `Option<String>`. The three host-interpolating endpoints (`S3`,
`S3DualStack`, `DigitalOceanSpaces`) require it: a missing region surfaces as
`Error::MissingField { field: "region" }` from `build()`. `GCS` and `Generic` never
read the region and build without it (under `s3-auth`, SigV4 still defaults the signing
region to `us-east-1` when none is set).

### Scheme validation for Generic endpoints (S4)

Both builders reject a `Generic` endpoint URL whose scheme is `http` by default, emitting
`Error::Config` with a message naming the rejected URL and pointing to
`allow_insecure_http`. Call `.allow_insecure_http(true)` on either builder to permit plain
HTTP (intended for localhost stubs and integration tests where TLS is unavailable). Named
endpoints (`S3`, `S3DualStack`, `GCS`, `DigitalOceanSpaces`) are unaffected: they always
produce `https` URLs. The `allow_insecure_http` field defaults to `false` on both builders.

The shared helper `backends::common::validate_url_scheme(url, allow_http) -> Result<()>`
performs the check and is exported `pub(crate)` for other backends (git shards) to reuse.

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

Parsing uses `quick_xml::Reader` with `trim_text(true)` and walks the `ListBucketResult`:

- A `<Contents>` start flushes any in-progress release via `add_to_releases_list`
  and resets state.
- `<Key>` text is matched against the filename regex (below); on match it sets the
  current release's `name`/`version`, forms the download URL as `download_base_url + key`,
  and sets a single-element `assets` vec whose `name` is the key's filename component
  (path stripped via `PathBuf::file_name`). A non-matching key is debug-logged and skipped.
- `<LastModified>` text sets the release `date`.
- `Eof` flushes the final in-progress release.

`add_to_releases_list` drops any release with an empty `name` or `version`, and merges
entries sharing the same `name`+`version` into one release with their assets concatenated;
otherwise it pushes a new release.

### Version derivation

A single case-insensitive regex parses object keys:
`(?i)(?P<prefix>.*/)*(?P<name>.+)-[v]{0,1}(?P<version>\d+\.\d+\.\d+)-.+`.
The key must contain a `name-[v]<major>.<minor>.<patch>-<suffix>` shape: `name`
becomes the release name and the dotted triple becomes the version, with any
leading `v` stripped. Keys lacking this shape produce no release.

`ReleaseUpdate` selection helpers operate on the parsed list: `pick_latest` picks the
highest version (ignoring unparseable ones, erroring `Error::NoReleaseFound` when empty);
`sort_newer` filters to strictly newer-than-current, newest-first; `find_version` matches
an exact version, erroring `Error::NoReleaseFound` when absent. These back
`get_latest_release`, `get_latest_releases`, and `get_release_version` and their `async`
siblings.

### Signing under s3-auth

The `auth` module is gated on `feature = "s3-auth"`. `AccessKey` is `#[non_exhaustive]`
with **private** fields `access_key_id` and `secret_access_key`; read them via the
accessor methods `access_key_id(&self) -> &str` and `secret_access_key(&self) -> &str`.
Build `AccessKey` through `AccessKey::new(access_key_id, secret_access_key)` (both args
`impl Into<String>`) or the `From<(&str, &str)>` / `From<(String, String)>` impls; it is
re-exported as `self_update::backends::s3::AccessKey`. The `#[non_exhaustive]` attribute
reserves room for a future STS session token; no `session_token` field exists today. The
`Debug` impl redacts `secret_access_key` (shows `<redacted>`).

`s3_signature_v4` implements AWS SigV4 presigned-query signing. With no `AccessKey` it
returns the URL unchanged -- so public buckets are unsigned. With one it appends
`X-Amz-Algorithm=AWS4-HMAC-SHA256`, `X-Amz-Credential`, `X-Amz-Date`, `X-Amz-Expires`,
`X-Amz-SignedHeaders=host`, and the `X-Amz-Signature` (lowercase hex HMAC-SHA256). Region
defaults to `us-east-1` when absent; the service is fixed to `s3` and the payload to
`UNSIGNED-PAYLOAD`. Both the listing URL (TTL 300s default) and each asset download URL
(TTL 300s default) are signed when an access key is present.

**Canonical host with non-default port**: The `canonical_host` helper builds the `host:`
canonical header value. For default-port URLs (https:443, http:80) it returns the hostname
only. For a URL with an explicit non-default port (e.g. `https://minio.local:9000/...`) it
returns `hostname:port`, matching the `Host:` header the HTTP client sends. This ensures
`SignatureDoesNotMatch` is not returned for non-standard port endpoints.

**Credential redaction in logs**: `redact_signed_url` strips the values of
`X-Amz-Signature` and `X-Amz-Credential` from a URL before it is emitted to the debug
log (replacing them with `<redacted>`). Both the listing URL (`s3_page`) and the matched
release asset URLs (`parse_s3_response`) use this helper for debug output. On unsigned
URLs (no `X-Amz-*` params) the URL is returned unchanged.

The s3 backend does not authenticate via bearer token. The shared `auth_token`
setter is omitted via `impl_common_builder_setters!(no_auth_token)`; there is no
`auth_token` method at all on the s3 builders (use `.access_key((id, secret))`
under `s3-auth`). The shared auth derivation is a no-op for s3 (no `Authorization`,
no `User-Agent`): `api_headers` uses the `UpdateConfig` trait default, which is a
no-op, because s3 authenticates by SigV4-signing the URL, not via an auth header.

### Errors

A non-2xx listing response is always an `Err`, never an `Ok` parsed from the error
body: `send` / `http_client::get` bail on any non-2xx status before returning. Both
clients map a completed non-2xx to the same structured variant by status: 404 ->
`Error::NotFound`, 401/403 -> `Error::Unauthorized`, any other non-2xx ->
`Error::HttpStatus`; a request that cannot complete (connection/TLS/timeout) is
`Error::Transport`. XML parse errors surface as `Error::InvalidResponse` with the
underlying quick-xml error chained via `source()`. Missing region (for the
region-requiring endpoints) and missing bucket are `Error::MissingField`. An `http`
scheme on a Generic endpoint (without `allow_insecure_http`) is `Error::Config`.

## Public surface

- `s3::Endpoint` (`#[non_exhaustive]`, `Default = S3`) with variants `S3`,
  `S3DualStack`, `GCS`, `DigitalOceanSpaces`, `Generic(String)`; plus
  `From<&str>` / `From<String>` -> `Generic` (always, regardless of string content).
- `s3::ReleaseList`, `s3::ReleaseListBuilder` (setters: `bucket_name`, `asset_prefix`,
  `region`, `endpoint`, `filter_target`, `max_keys`, `allow_insecure_http`,
  `access_key` [s3-auth], `signature_ttl` [s3-auth], request-config setters, `build`).
- `s3::UpdateBuilder`, `s3::Update` (`#[non_exhaustive]`); `Update::configure`,
  `build` -> `Box<dyn ReleaseUpdate>`, `build_async` -> `Update` [async]. Builder setters
  include `allow_insecure_http`.
- `s3::AccessKey` [s3-auth], re-exported, `#[non_exhaustive]`, private fields,
  `AccessKey::new`, tuple `From` impls, and accessor methods `access_key_id()` /
  `secret_access_key()`. `Debug` redacts the secret.

## Invariants and regression checklist

- `bucket_name` required on both builders -> `Error::MissingField`.
- Region required for `S3`/`S3DualStack`/`DigitalOceanSpaces`; ignored for
  `GCS`/`Generic`. Missing required region -> `Error::MissingField`.
- `Endpoint::Generic` with an `http` scheme is rejected at `build()` unless
  `allow_insecure_http(true)` is set -> `Error::Config`. Named endpoints always use https
  and are unaffected.
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
  both listing and asset URLs are SigV4-signed (TTL 300s default), region defaulting
  to `us-east-1`.
- Under `s3-auth`, the canonical host header includes `hostname:port` for non-default
  ports, matching the HTTP client's `Host:` header to avoid `SignatureDoesNotMatch`.
- Signed URLs are redacted before debug logging: `X-Amz-Signature` and
  `X-Amz-Credential` values are replaced with `<redacted>`.
- `AccessKey` fields are private; read via `access_key_id()` / `secret_access_key()`.
  `Debug` shows `access_key_id` and `<redacted>` for the secret.
- `auth_token` is absent on the s3 builders (use `.access_key(...)` instead);
  `api_headers` sets no `User-Agent`.
- `Endpoint`, `Update`, and `AccessKey` are `#[non_exhaustive]`.

## Tests

In-module tests: `parse_s3_response` cases (single/multi asset, v-prefix strip, multiple
releases, non-matching-key skip, path-stripped filename, malformed-XML error, empty body);
`add_to_releases_list` empty-name/version drop; loopback-TCP stub tests for the sync and
async `ReleaseUpdate` fetch methods (`get_latest_release`, `get_latest_releases`,
`get_release_version`, `is_update_available`, multi-asset merge); the non-2xx error
contract; `pick_latest`/`sort_newer`/`find_version` unit tests; `build_s3_api_url` shape
tests per endpoint, prefix append, and missing-region error; and (under `s3-auth`)
SigV4 conformance vectors, structural invariants, region default, listing/asset URL
signing, TTL threading.

New tests added in this change:
- `redact_signed_url_masks_sensitive_params` — asserts `X-Amz-Signature` and
  `X-Amz-Credential` values are replaced with `<redacted>` while other params pass through.
- `redact_signed_url_passes_unsigned_url_through_unchanged` / `_no_query_string_is_unchanged`
  — unsigned URLs returned verbatim.
- `canonical_host_includes_port_for_non_default_port` [s3-auth] — non-default port produces
  `host:port`; default port 443 / no port produces hostname only.
- `signer_uses_canonical_host_with_port_for_generic_non_default_endpoint` [s3-auth] —
  the canonical request for a `:9000` URL embeds `host:minio.local:9000`.
- `access_key_accessors_return_correct_values` [s3-auth] — `access_key_id()` /
  `secret_access_key()` return correct values.
- `access_key_debug_redacts_secret` [s3-auth] — `Debug` shows key ID, hides secret.
- `http_generic_endpoint_rejected_by_default_on_update_builder` /
  `_on_release_list_builder` — `http` scheme rejected as `Error::Config` by default.
- `http_generic_endpoint_allowed_after_opt_in_on_update_builder` /
  `_on_release_list_builder` — `allow_insecure_http(true)` permits plain http.
- `https_generic_endpoint_always_allowed_on_update_builder` — https always OK.
- `named_endpoints_are_unaffected_by_scheme_validation` — GCS/S3 build without errors.
- `update_builder_invalid_request_header_surfaces_at_build` — end-to-end deferred-header
  error surfaces as `Error::InvalidHeader` from `build()`.
- `access_key_is_reexported_and_built_from_tuples` updated to use accessors (not fields).

## Related

- `s3-auth-token-removal.md`
- `s3-max-keys-configurable.md`
- `transport-control.md`
- `error-network-vs-http-semantics.md`
- `async-api.md`

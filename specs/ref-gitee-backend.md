# Gitee backend (reference)

Status: implemented

## Scope

Canonical description of the Gitee release backend in `src/backends/gitee.rs`. It
covers the `ReleaseList` query builder, the `Update` / `AsyncUpdate` builders, the
Gitee REST API v5 route shapes, authentication, pagination, the latest-release
endpoint and its non-semver fallback, the JSON-to-model mapping with its deliberate
lenient-asset handling, and error mapping. Sync and async paths are documented
together; the async surface is gated behind the `async` feature.

## Behavior

### Builders

Two builders exist, each reached through a `configure()` constructor:

- `ReleaseList::configure()` returns `ReleaseListBuilder`. The builder holds `host`,
  `repo_owner`, `repo_name`, `target`, `auth_token`, and a `RequestConfig`. It is
  `#[derive(Clone, Debug)]` and `#[must_use]`. `build()` validates and returns a
  `ReleaseList`.
- `Update::configure()` returns `UpdateBuilder`. The builder holds `host`,
  `repo_owner`, `repo_name`, and a `CommonBuilderConfig`; it is
  `#[derive(Clone, Debug, Default)]` and `#[must_use]`. `UpdateBuilder::new()` is
  `Default::default()`. The common setters (target, bin_name, current_version,
  auth_token, request headers, etc.) come from `impl_common_builder_setters!()`.

`UpdateBuilder` exposes two terminal methods:

- `build()` returns the concrete `Update`.
- `build_async()` (feature `async`) also returns the concrete `Update`, with the
  inherent `*_async` methods reachable. There is no separate async builder type.

`Update` is `Send` and exposes the update verbs as inherent methods (`update`,
`update_extended`, `get_latest_release`, `get_newer_releases`, `get_release_version`,
`is_update_available`), so no trait import is needed. Both terminal methods delegate
to the private `build_update()` helper (see below). `ReleaseList` has `fetch` and,
under `async`, `fetch_async`, both returning `Result<Releases>`.

### Route shapes and host

- The default host is `https://gitee.com`. The `host(impl Into<String>)` setter on
  both builders is optional and allows pointing at a Gitee enterprise instance
  (scheme + host only, no trailing slash, no `/api/v5`). The crate appends the
  `/api/v5/...` path itself.
- The base releases route is `<host>/api/v5/repos/<owner>/<repo>/releases`.
- The latest-release route is `<host>/api/v5/repos/<owner>/<repo>/releases/latest`
  (returns a single object). When the latest release's `tag_name` is not a valid
  semver string, the backend falls back to scanning the listing endpoint to find the
  highest semver release; non-semver tags are skipped with a debug log during the
  scan (parity with the other forges).
- Fetch-by-tag uses `<host>/api/v5/repos/<owner>/<repo>/releases/tags/{tag}` with
  the tag percent-encoded via `urlencoding::encode`. This route returns a single
  object (not an array).
- Unlike Gitea, a `host` value is not required at build time because `https://gitee.com`
  is the well-known default. `build()` / `build_update()` only error on missing
  `repo_owner` or `repo_name`.

### Auth

- `auth_token` is set on `ReleaseListBuilder` via `auth_token(impl Into<String>)`; on
  `UpdateBuilder` it comes through the common setters and is stored in `CommonConfig`.
- Auth is applied centrally by `apply_auth` (`common.rs`), which renders the token as
  `Authorization: Bearer <token>`. This matches the scheme used by Gitee's official
  client (oschina/mcp-gitee `utils/gitee_client.go:172`, verified 2026-07-17).
- The header is marked sensitive and is never logged and never placed in URLs.
- The token is host-gated: it is only attached to requests whose host matches the
  configured instance host (or an `allow_auth_host` entry), over https.
  `dangerously_allow_non_https_auth_forwarding()` relaxes the https requirement, and a
  user-set `Authorization` via `request_header` overrides it.
- A token that cannot parse into a header value surfaces as `Error::InvalidAuthToken`.
- Headers always include `User-Agent: rust-reqwest/self-update`.

### Pagination and ordering

- Listing uses `Link: rel="next"` pagination via the sans-io core: a `PageRequest`
  whose parser maps each page via an array handler and follows `next_link(headers)`,
  driven by `run_paginated` / `run_paginated_async` (`backends/mod.rs`). The first
  page URL appends `?per_page=100` when no query string is present. Pagination is
  bounded by `MAX_RELEASE_PAGES` (100) in the driver.
- The latest-release path hits `/releases/latest` directly and does not paginate. The
  response is a single object. When that object's `tag_name` fails semver parsing, the
  backend falls back to the listing scan described above.
- The newer-releases paths (`get_newer_releases` / `get_newer_releases_async`) fold the
  strictly-newer filter into the plan: with `stop_at = Some(current_version)`, the
  parser keeps releases where `bump_is_greater(current, version)` is true and skips
  the rest per-item; pagination continues through all pages regardless.
  `ReleaseList::fetch` passes `stop_at = None` and walks all pages unfiltered.

### JSON to model

**Asset leniency (deliberate divergence from github/gitlab/gitea).** Gitee's v5 API
always includes auto-generated source archive entries in the `assets` array. These
entries have no `name` and no `browser_download_url`. If the backend applied the same
strict handling used by the other forge backends, every release would fail to parse
because the nameless source archive entry would trigger `Error::MissingAssetField`.
To avoid this, the Gitee backend uses a lenient asset strategy:

- An asset entry that is missing `name` OR missing `browser_download_url` is silently
  skipped with a debug log (`"skipping asset missing name or download url"`).
- No `Error::MissingAssetField` is raised for an individual asset. The release still
  parses successfully as long as at least one valid (named, URL-bearing) asset exists.
- This is a deliberate, documented divergence from the strict DTO handling in
  `github.rs`, `gitlab.rs`, and `gitea.rs`.

Release-level field mapping:

- `tag_name` (required, else `Error::MissingAssetField { field: "tag_name" }`) ->
  `version` with a single leading `v` stripped via `trim_start_matches('v')`. A
  `tag_name` that does not parse as semver causes the release to be skipped with a
  debug log during listing/latest scans; it surfaces as an error only when directly
  fetched by tag (which implies the caller already knows the exact tag).
- `created_at` (required, else `Error::MissingAssetField { field: "created_at" }`) ->
  `date`.
- `name` -> `name`, defaulting to the tag when absent.
- `assets` -> zero or more asset entries filtered through the lenient strategy above.
- `body` -> optional `body` (`None` when absent or non-string).

`get_release_version[_async]` parses the bare object returned by `/tags/{tag}`
directly (not wrapped in an array), while the list and latest endpoints require their
respective shapes.

### tag_prefix support

`tag_prefix` is wired through the shared machinery. When set, the backend strips the
prefix from `tag_name` before semver parsing and from the version string stored in the
`Release` model.

### Errors

- Missing `repo_owner` or `repo_name` at build time -> `Error::MissingField { field }`
  with `field` naming the missing setter. `host` has a default and is not required.
- A deferred `request_header` conversion failure surfaces from `build()` via
  `CommonBuilderConfig::build` as `Error::InvalidHeader`.
- An empty releases array -> `Error::NoReleaseFound { target: None }`; a non-array
  list payload -> `Error::InvalidResponse { source }` (the serde_json error is chained).
- A missing required JSON field (tag_name or created_at) ->
  `Error::MissingAssetField { field }`.
- A token that cannot parse into a header value -> `Error::InvalidAuthToken` (via
  `apply_auth`).
- Transport/HTTP failures propagate from the shared `send` / `send_async` helpers.

### `build_update` helper

`UpdateBuilder::build_update` is the private validator shared by `build` and
`build_async`. It resolves `host` (filling in the default when unset), `repo_owner`,
and `repo_name` (erroring as above) and calls `self.common.build()?` to produce the
`CommonConfig`, returning a concrete `Update`. Keeping it private ensures the sync
and async terminal methods validate identically and cannot drift.

## Public surface

- `gitee::ReleaseList`, `gitee::ReleaseListBuilder`
  - `ReleaseList::configure() -> ReleaseListBuilder`
  - `ReleaseListBuilder`: `host`, `repo_owner`, `repo_name`, `filter_target`,
    `auth_token`, the `request_config_setters!` setters, `build`
  - `ReleaseList::fetch() -> Result<Releases>` (filters by `target` when set; returns
    a `Releases` whose `current_version()` is `None`, so recover the `Vec<Release>`
    with `into_vec()`); `ReleaseList::fetch_async()` (feature `async`)
- `gitee::Update` (`#[non_exhaustive]`), `gitee::UpdateBuilder`
  - `Update::configure() -> UpdateBuilder`
  - `UpdateBuilder`: `new`, `host`, `repo_owner`, `repo_name`, common setters,
    `build`, `build_async` (feature `async`); both return the concrete `Update`
  - `Update` is `Send`, exposes the inherent verbs (`update`, `update_extended`,
    `get_latest_release`, `get_newer_releases`, `get_release_version`,
    `is_update_available`), and implements `ReleaseUpdate` (sync) and the public sealed
    `AsyncReleaseUpdate` (feature `async`, including `get_newer_releases_async`)
- Free `api_headers` and the plan builders are private to the module.

`Update` is `#[non_exhaustive]` so its fields stay private and future fields do not
break downstream code; it is constructed only through the builder.

## Risk note

CI has no live Gitee network access. Correctness rests on the documented v5 API
contract and on loopback stub tests that pin that contract. Any change to Gitee's
actual v5 response shapes (e.g. if they stop including nameless source archive entries,
or change the latest-release endpoint shape) would not be caught by CI automatically.

## Invariants and regression checklist

- Tag is percent-encoded in the fetch-by-tag route via `urlencoding::encode`.
- Base route shape is exactly `<host>/api/v5/repos/<owner>/<repo>/releases`, shared by
  sync, async, and `ReleaseList` paths.
- Default host is `https://gitee.com`; `host` setter is optional.
- Latest-release path uses `/releases/latest` (a dedicated endpoint, unlike Gitea). A
  non-semver result from that endpoint triggers a listing-scan fallback.
- Auth uses `Authorization: Bearer <token>` (not `token <token>`), matching the Gitee
  v5 API. Token is never logged, never in URLs, host-gated to the configured instance.
- Nameless or URL-less assets are skipped with a debug log, not an error. This is
  intentional and must not be "fixed" to strict handling without understanding the
  source-archive implication.
- `tag_name` and `created_at` remain required at the release level.
- Non-semver `tag_name` values cause the release to be skipped during scans (debug log).
- `version` has a single leading `v` stripped; `name` defaults to the tag.

## Tests

In `src/backends/gitee.rs` `mod tests`, backed by a loopback `TcpListener` stub (no
external network):

- **Nameless-asset skip**: a release whose assets array contains one nameless entry is
  parsed successfully; the nameless entry is absent from the resulting asset list.
- **Non-semver skip (listing)**: releases with non-semver `tag_name` are excluded from
  listing results; a listing containing only non-semver tags surfaces as
  `Error::NoReleaseFound`.
- **Non-semver skip (latest)**: the `/releases/latest` response returns a non-semver
  tag; the backend falls back to the listing scan and returns the highest semver
  release found there.
- **Non-semver skip (pinned tag)**: a direct `/releases/tags/{tag}` fetch for a
  non-semver tag surfaces as `Error::MissingAssetField { field: "tag_name" }` or an
  equivalent parse error (not silently skipped, since the caller named the tag
  explicitly).
- **Pagination**: two-page listing via `Link: rel="next"` returns releases from both
  pages; first page appends `?per_page=100`.
- **Auth threading + token-never-logged**: auth token appears in the
  `Authorization: Bearer` header on the outgoing request; no log line at any level
  contains the token string.
- **Empty response**: an empty JSON array from the listing endpoint surfaces as
  `Error::NoReleaseFound`.
- **Malformed response**: a non-array payload from the listing endpoint surfaces as
  `Error::InvalidResponse`.

## Related

- `release-tag-url-encoding.md` (percent-encoding of the fetch-by-tag route)
- `transport-control.md` (request headers, timeout, retries, client override)
- `ref-release-model.md` (the `Release` / `ReleaseAsset` model these map into)
- `release-scan-pagination.md` (the shared `Link: rel="next"` pagination)
- `choose-latest-release-sort.md` (newest-first ordering assumptions)
- `ref-gitea-backend.md` (Gitea backend, which this parallels with noted divergences)

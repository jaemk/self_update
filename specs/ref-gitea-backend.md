# Gitea backend (reference)

Status: implemented

## Scope

Canonical description of the Gitea release backend in `src/backends/gitea.rs`. It
covers the `ReleaseList` query builder, the `Update` / `AsyncUpdate` builders, the
Gitea REST API route shapes, authentication, pagination, single-newest ordering, the
JSON-to-model mapping, the private `build_update` helper, and error mapping. Sync and
async paths are documented together; the async surface is gated behind the `async`
feature.

## Behavior

### Builders

Two builders exist, each reached through a `configure()` constructor:

- `ReleaseList::configure()` returns `ReleaseListBuilder` (`gitea.rs:163`). The
  builder holds `host`, `repo_owner`, `repo_name`, `target`, `auth_token`, and a
  `RequestConfig` (`gitea.rs:62-69`). It is `#[derive(Clone, Debug)]` and `#[must_use]`.
  `build()` validates and returns a `ReleaseList` (`gitea.rs:122-147`).
- `Update::configure()` returns `UpdateBuilder` (`gitea.rs:297-299`). The builder
  holds `host`, `repo_owner`, `repo_name`, and a `CommonBuilderConfig`
  (`gitea.rs:200-205`); it is `#[derive(Clone, Debug, Default)]` and `#[must_use]`.
  `UpdateBuilder::new()` is `Default::default()` (`gitea.rs:209-211`). The common
  setters (target, bin_name, current_version, auth_token, request headers, etc.) come
  from `impl_common_builder_setters!()` (`gitea.rs:236`).

`UpdateBuilder` exposes two terminal methods:

- `build()` returns `Box<dyn ReleaseUpdate>` for the sync API (`gitea.rs:267-269`).
- `build_async()` (feature `async`) returns the concrete `Update` so the inherent
  `*_async` methods are reachable (`gitea.rs:275-278`). This is the "AsyncUpdate"
  entry point; there is no separate async builder type.

Both delegate to the private `build_update()` helper (see below).

### Route shapes and host

- The base releases route is built by `Update::releases_url()` (`gitea.rs:302-307`)
  and, identically, inline in `ReleaseList::fetch` (`gitea.rs:177-180`):
  `<host>/api/v1/repos/<owner>/<repo>/releases`.
- Fetch-by-tag appends `/tags/{tag}` to that base, with the tag percent-encoded via
  `urlencoding::encode(ver)`: `<base>/tags/{encoded_tag}`
  (`gitea.rs:364` sync, `gitea.rs:472` async).
- The custom host is set with `url(host)` on both builders (`gitea.rs:77-80`,
  `gitea.rs:219-222`). The method carries `#[doc(alias = "instance_url")]` and
  `#[doc(alias = "with_host")]` so it is discoverable under the old `instance_url`
  name. Gitea has no canonical public host, so `url` is required: `build()` /
  `build_update()` `bail!` with `Error::Config` when it is unset
  (`gitea.rs:128-132`, `gitea.rs:244-248`).
- Unlike GitHub, Gitea has no dedicated `/releases/latest` endpoint, so "latest" is
  derived from the list endpoint (see ordering below) (`gitea.rs:328-331`).

### Auth

- `auth_token` is set on `ReleaseListBuilder` via `auth_token(&str)`
  (`gitea.rs:114-117`); on `UpdateBuilder` it comes through the common setters and is
  stored in `CommonConfig`.
- Headers are built by the free function `api_headers(auth_token)`
  (`gitea.rs:484-503`). It always sets `User-Agent: rust-reqwest/self-update`. When a
  token is present it adds `Authorization: token <token>` (the Gitea `token` scheme,
  not `Bearer`). A token that cannot parse into a header value maps to `Error::Config`
  (`gitea.rs:497-499`).
- The `Update`'s `UpdateConfig` accessor override wires this same `api_headers`
  via `impl_update_config_accessors!` (`gitea.rs:375-379`), so the trait default
  (which sets no User-Agent) is not used.

### Pagination and ordering

- Listing follows Gitea's `Link: rel="next"` pagination. `fetch_all_releases`
  (`gitea.rs:382-399`) and its async sibling `fetch_all_releases_async`
  (`gitea.rs:404-429`) drive `collect_paginated` / `collect_paginated_async` starting
  from `first_page_url(base)` (which appends `?per_page=100` when no query is present).
  Each page is parsed and the next URL comes from `next_link(headers)`. Pagination is
  bounded by `MAX_RELEASE_PAGES` (100) in the shared helper.
- "Single newest" is `releases[0]` of the first page; the code relies on the list
  endpoint's default descending (newest-first) order rather than sorting
  (`gitea.rs:314-332` sync `fetch_latest_release`, `gitea.rs:433-452` async
  `get_latest_release_async`). The latest path does not paginate; it reads only the
  first response.
- The newer-releases paths fetch the full paginated list, then filter to releases
  strictly newer than `current_version` using `bump_is_greater(...).unwrap_or(false)`,
  preserving source order (`gitea.rs:336-347` sync, `gitea.rs:454-468` async).
  Filtering happens after pagination, so a newer release on a later page is retained.

### JSON to model

- `Release::from_release_gitea` (`gitea.rs:33-56`) maps one release object:
  - `tag_name` (required, else `Error::Release`) -> `version` with a single leading
    `v` stripped via `trim_start_matches('v')`.
  - `created_at` (required, else `Error::Release`) -> `date`.
  - `name` -> `name`, defaulting to the tag when absent (`gitea.rs:40`).
  - `assets` (required array, else `Error::Release` "No assets found") -> each mapped
    by `ReleaseAsset::from_asset_gitea`.
  - `body` -> optional `body` (`None` when absent or non-string).
- `ReleaseAsset::from_asset_gitea` (`gitea.rs:18-29`) requires `browser_download_url`
  -> `download_url` and `name` -> `name`; either missing is `Error::Release`.
- `get_release_version[_async]` parses the bare object returned by `/tags/{tag}`
  directly (not wrapped in an array) (`gitea.rs:371`, `gitea.rs:480`), while the list
  endpoints parse a JSON array.

### Errors

- Missing/invalid host, owner, or name at build time -> `Error::Config`
  (`gitea.rs:128-141`, `gitea.rs:244-257`).
- A deferred `request_header` conversion failure surfaces from `build()` via
  `request.check()` / `CommonBuilderConfig::build` as `Error::Config`
  (`gitea.rs:123`, `gitea.rs:259`).
- A non-array list payload, an empty releases array, or any missing required JSON
  key -> `Error::Release` (`gitea.rs:322-327`, `gitea.rs:444-449`, plus the parser
  guards above).
- A token that cannot parse into a header value -> `Error::Config`
  (`gitea.rs:497-499`).
- Transport/HTTP failures propagate from the shared `send` / `send_async` helpers.

### `build_update` helper

`UpdateBuilder::build_update` (`gitea.rs:239-261`) is the private validator shared by
`build` and `build_async`. It resolves `host` / `repo_owner` / `repo_name` (erroring
as above) and calls `self.common.build()?` to produce the `CommonConfig`, returning a
concrete `Update`. Keeping it private ensures the sync and async terminal methods
validate identically and cannot drift.

## Public surface

- `gitea::ReleaseList`, `gitea::ReleaseListBuilder`
  - `ReleaseList::configure() -> ReleaseListBuilder`
  - `ReleaseListBuilder`: `url`, `repo_owner`, `repo_name`, `filter_target`,
    `auth_token`, the `request_config_setters!` setters, `build`
  - `ReleaseList::fetch() -> Result<Vec<Release>>` (filters by `target` when set)
- `gitea::Update` (`#[non_exhaustive]`), `gitea::UpdateBuilder`
  - `Update::configure() -> UpdateBuilder`
  - `UpdateBuilder`: `new`, `url`, `repo_owner`, `repo_name`, common setters,
    `build`, `build_async` (feature `async`)
  - `Update` implements `ReleaseUpdate` (sync) and `AsyncFetch` (feature `async`)
- Free `api_headers` and the `fetch_all_releases[_async]` helpers are private to the
  module.

`Update` is `#[non_exhaustive]` (`gitea.rs:288`) so its fields stay private and future
fields do not break downstream code; it is constructed only through the builder.

## Invariants and regression checklist

- Tag is percent-encoded in the fetch-by-tag route via `urlencoding::encode`
  (`gitea.rs:364`, `gitea.rs:472`).
- Base route shape is exactly `<host>/api/v1/repos/<owner>/<repo>/releases`, shared by
  sync, async, and `ReleaseList` paths via `releases_url()` (`gitea.rs:302-307`).
- "Latest" is `releases[0]` of the first page, depending on the endpoint's newest-first
  ordering; the latest path does not paginate (`gitea.rs:331`, `gitea.rs:450`).
- Newer-release filtering is strict (`bump_is_greater`), runs after pagination, and
  preserves source order.
- `url` is required (no default host); missing it is `Error::Config`.
- Auth uses the `token <token>` scheme with a fixed `rust-reqwest/self-update`
  User-Agent.
- `version` has a single leading `v` stripped; `name` defaults to the tag.

## Tests

In `src/backends/gitea.rs` `mod tests` (`gitea.rs:505-1126`), backed by a loopback
`TcpListener` stub (no external network):

- Sync `ReleaseUpdate` fetch: one-element latest wrap, strictly-newer filtering,
  no-update-when-up-to-date, and single-vs-list agreement
  (`gitea.rs:616-742`).
- Builder shape: `url`/`filter_target` exist on `ReleaseListBuilder`; `ReleaseList`
  and `Update` builds require `url`, `repo_owner`, `repo_name`; invalid header surfaces
  as `Error::Config`; `releases_url` shape; identifier and `bin_name` wiring
  (`gitea.rs:744-916`).
- `api_headers` override uses the Gitea User-Agent and `token` scheme
  (`gitea.rs:767-797`).
- Async (feature `async`): latest parse, `Link` pagination across two pages,
  `/tags/{ver}` single-object parse, missing-`tag_name` error, newer-only filtering,
  empty-when-up-to-date, accumulate-then-filter across pages, empty-array error,
  non-array-payload error (`gitea.rs:918-1125`).

## Related

- `release-tag-url-encoding.md` (percent-encoding of the fetch-by-tag route)
- `transport-control.md` (request headers, timeout, retries, client override)
- `ref-release-model.md` (the `Release` / `ReleaseAsset` model these map into)
- `release-scan-pagination.md` (the shared `Link: rel="next"` pagination)
- `choose-latest-release-sort.md` (newest-first ordering assumptions)

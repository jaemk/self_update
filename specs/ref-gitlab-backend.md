# GitLab backend (reference)

Status: implemented

## Scope

Canonical reference for the GitLab release backend in `src/backends/gitlab.rs`. It
documents the `ReleaseList` listing builder, the `Update` / async update builders, the
GitLab API v4 route shapes, authentication, pagination, ordering, the JSON-to-model
mapping, and error mapping. Every claim is verified against `gitlab.rs`. Shared
pagination/transport helpers (`send`, the sans-io `PageRequest`/`Page` core and its
`run_paginated` / `run_paginated_async` drivers, `first_page_url`, `next_link`) live in
`src/backends/mod.rs`; common builder/config plumbing lives in `src/backends/common.rs`.

## Behavior

### Builders

`ReleaseList` lists releases for a repo and returns `Vec<Release>`. It is configured via
`ReleaseList::configure()` (`gitlab.rs:160`), which seeds `host` to `https://gitlab.com`
(`gitlab.rs:162`). The builder (`ReleaseListBuilder`, `gitlab.rs:63`) exposes `url`,
`repo_owner`, `repo_name`, `filter_target`, `auth_token`, the shared
`request_config_setters!(request)` setters (`gitlab.rs:117`), and `build()`
(`gitlab.rs:120`). `build()` calls `self.request.check()` first (surfacing any deferred
`request_header` error as `Error::Config`), then requires `repo_owner` and `repo_name`,
each bailing `Error::Config` when unset (`gitlab.rs:124-139`). The required-field messages
name the setter to call: `` `repo_owner` required (call `.repo_owner(...)`) `` and
`` `repo_name` required (call `.repo_name(...)`) `` (`gitlab.rs:129`, `gitlab.rs:137`).
`repo_owner`/`repo_name` are stored `Option<String>` on the builder and resolved to
`String` on `ReleaseList`.

`filter_target` (`gitlab.rs:101`) sets a target that drops whole releases lacking a
matching asset; it is the `ReleaseList` release filter. `fetch()` applies it via
`r.has_target_asset(target)` (`gitlab.rs:181-186`). This differs from `Update::target`,
which selects *which asset* to download.

`Update` is built via `Update::configure()` -> `UpdateBuilder` (`gitlab.rs:198`,
`gitlab.rs:283`). Backend-specific setters are `url`, `repo_owner`, `repo_name`; all common
options come from `impl_common_builder_setters!()` (`gitlab.rs:233`). `build()` returns
`Box<dyn ReleaseUpdate>` (`gitlab.rs:262`); under the `async` feature `build_async()`
returns the concrete `Update` so the inherent `*_async` methods are reachable
(`gitlab.rs:271`). Both delegate to `build_update()` (`gitlab.rs:235`), which requires
`repo_owner`/`repo_name` (each bailing `Error::Config` with the same setter-naming messages,
`gitlab.rs:243`, `gitlab.rs:251`) and calls `self.common.build()` (which runs the
deferred-header `check` and validates `current_version`, `bin_name`, `bin_path_in_archive`).
`UpdateBuilder::default()` seeds `host` to `https://gitlab.com` (`gitlab.rs:378-386`).

### Route shapes, host, and project-path encoding

The list/latest/newer routes share one base, `Update::releases_url()` (`gitlab.rs:297`),
and `ReleaseList::fetch` builds the same shape (`gitlab.rs:174`):

```
<host>/api/v4/projects/<owner>%2F<repo>/releases
```

The literal `%2F` separating owner and repo is hard-coded in the format string
(`gitlab.rs:175-178`, `gitlab.rs:298-303`); only `repo_owner` is run through
`urlencoding::encode`, while `repo_name` is interpolated verbatim. Encoding `repo_owner`
matters for subgroup paths (e.g. `group/subgroup` becomes `group%2Fsubgroup`) so an
embedded `/` does not create an extra path segment.

Fetch-by-tag (`get_release_version` / `get_release_version_async`) appends the tag to the
releases base, percent-encoding the tag (`gitlab.rs:361`, `gitlab.rs:480`):

```
{releases_url}/{urlencoding::encode(tag)}
```

This route returns a single release *object* (not an array), parsed directly by
`Release::from_release_gitlab`.

Custom host: `url(impl Into<String>)` (`gitlab.rs:77`, `gitlab.rs:216`) overrides `host`. The
setter was renamed from `instance_url`/`with_host` to `url`; it carries no `#[doc(alias)]`
(all builder-setter doc-aliases were dropped). Its doc states the instance host only (scheme
+ host, no trailing slash and no `/api/v4`): the crate appends the `/api/v4/...` path itself
(`gitlab.rs:75-76`, `gitlab.rs:214-215`), so callers pass e.g. `https://gitlab.example.com`.
The string setters
(`url`, `repo_owner`, `repo_name`, `filter_target`, `auth_token`, and the `Update` builder's
common setters) take `impl Into<String>`.

### Authentication

`api_headers(auth_token: Option<&str>)` (`gitlab.rs:492`) always sets
`User-Agent: rust-reqwest/self-update` and, when a token is present, inserts
`Authorization: Bearer <token>` (`gitlab.rs:501-508`). A token that cannot be parsed into a
header value yields `Error::Config` ("Failed to parse auth token"). There is no
`PRIVATE-TOKEN` header and no environment-variable lookup in this file; the token comes
solely from the builder setter (`ReleaseListBuilder::auth_token`, `gitlab.rs:112`) or the
common `auth_token` setter for `Update` (`self.common.auth_token`). The
`impl_update_config_accessors!` override arm wires this `api_headers` into the trait so the
download path uses the Bearer scheme rather than the trait default `token` scheme
(`gitlab.rs:372-376`).

### Pagination and ordering

Listing paths (`ReleaseList::fetch`, `get_latest_releases`, and the async
`get_latest_releases_async`) build a transport-free `PageRequest<Release>` via
`releases_plan(base, auth, stop_at)` and drive it with the sans-io `run_paginated` /
`run_paginated_async` drivers (`backends/mod.rs`). The plan starts at `first_page_url(base_url)`
(which appends `?per_page=100` when the URL has no query string), parses each page with
`Release::from_release_gitlab`, and follows GitLab's `Link: rel="next"` (`next_link`).
`get_latest_releases` passes `stop_at = Some(current_version)` and the parser sets `Page::stop` on
the first release not strictly newer than it (early-stop, relying on newest-first order), while
`ReleaseList::fetch` passes `stop_at = None` and walks all pages. Pagination is bounded by
`MAX_RELEASE_PAGES` (100) in the driver; a still-advertised next page past that bound logs a
warning and stops.

Single-newest ordering: `get_latest_release` / `get_latest_release_async` use `newest_plan`, which
fetches just the first page and takes `releases[0]`. Unlike GitHub, GitLab has no dedicated
`/releases/latest` endpoint, so "newest" relies on the list endpoint's default descending
(newest-first) order. An empty array or a non-array payload yields `Error::Release` ("no releases
found").

Newer-than filtering is folded into the plan: with `stop_at = Some(current_version)`, the parser
keeps releases where `bump_is_greater(current_version, version)` is true and stops at the first that
is not, preserving order. The downstream `choose_latest_release` re-sort still selects the same
release, so early-stop and a full walk pick identically.

### JSON to model

`Release::from_release_gitlab` (`gitlab.rs:34`) maps a release object:
`tag_name` -> required `version` with a leading `v` trimmed (`trim_start_matches('v')`,
`gitlab.rs:52`); `created_at` -> required `date`; `name` -> `name`, defaulting to the tag
when absent (`gitlab.rs:41`); `description` -> optional `body` (`gitlab.rs:45`). Assets are
read from `assets.links` (not a bare `assets` array); a missing/non-array `assets.links`
yields `Error::Release` ("No assets found") (`gitlab.rs:42-44`). Each asset is parsed by
`ReleaseAsset::from_asset_gitlab` (`gitlab.rs:19`), which requires `url` (-> `download_url`)
and `name`, each bailing `Error::Release` when missing (`gitlab.rs:20-25`). Missing
`tag_name` or `created_at` also yields `Error::Release`.

### Errors

A completed non-2xx response is rejected by `send` / `send_async` before any body is parsed
and mapped to a structured variant by status (`status_to_error`, `errors.rs:254`): 404 ->
`Error::NotFound` (e.g. an unknown tag), 401/403 -> `Error::Unauthorized`, any other non-2xx
-> `Error::HttpStatus` (e.g. a 500/503 listing failure). A request that cannot complete is
`Error::Transport`. JSON shape and field problems (non-array listing payload, empty array on
the latest path, missing `tag_name`/`created_at`/`assets.links`, missing asset `url`/`name`)
surface as `Error::Release`. Builder/config problems (missing `repo_owner`/`repo_name`,
deferred bad `request_header`, unparseable auth token) surface as `Error::Config`.

## Public surface

- `ReleaseList::configure() -> ReleaseListBuilder`; `ReleaseList::fetch() -> Result<Vec<Release>>`.
- `ReleaseListBuilder`: `url`, `repo_owner`, `repo_name`, `filter_target`, `auth_token`,
  the `request_config_setters!` setters, `build() -> Result<ReleaseList>`.
- `Update::configure() -> UpdateBuilder`.
- `UpdateBuilder`: `new`, `url`, `repo_owner`, `repo_name`, the common setters,
  `build() -> Result<Box<dyn ReleaseUpdate>>`, and (feature `async`)
  `build_async() -> Result<Update>`.
- `Update` is `#[non_exhaustive]`, implements `ReleaseUpdate` (`get_latest_release`,
  `get_latest_releases`, `get_release_version`) and, under `async`, the public sealed
  `AsyncReleaseUpdate` (the `*_async` fetch verbs plus the default `update_async` /
  `update_extended_async`).
- `url` carries no `#[doc(alias)]`; all builder-setter doc-aliases were dropped.

## Invariants and regression checklist

- Project path is percent-encoded: `repo_owner` passes through `urlencoding::encode`; the
  `%2F` between owner and repo is literal in the route. A `/` in `repo_owner` must appear as
  `%2F` in the request line, never as a literal slash.
- Fetch-by-tag percent-encodes the tag (`urlencoding::encode(ver)`) before appending it to
  `{releases_url}/`.
- List route is exactly `<host>/api/v4/projects/<owner>%2F<repo>/releases`; tag route is
  `{releases_url}/{enc(tag)}`.
- Single-newest path takes `releases[0]` and depends on the list endpoint's descending
  order; empty/non-array payloads error rather than panic.
- Newer-than filtering runs after pagination, preserving order.
- Auth uses `Authorization: Bearer <token>` plus a fixed `User-Agent`; no `PRIVATE-TOKEN`
  header, no env var.
- `Update` is `#[non_exhaustive]`.

## Tests

In-file tests (`gitlab.rs:513-1420`) use a loopback `TcpListener` stub (no external
network). Coverage: sync and async latest/newer/by-tag parsing and version trimming;
`Link: rel="next"` pagination accumulation across two pages; newer-than filtering after
pagination; empty-array and non-array error paths; missing `tag_name` and missing
`assets.links` parser guards; non-2xx (404 -> `NotFound`, 500/503 -> `HttpStatus`);
`releases_url_encodes_repo_owner_with_slash` asserting `%2F` (and absence of a literal
`group/subgroup`) in the captured request line; `url`/`filter_target` setter existence;
`api_headers` Bearer + User-Agent wiring; invalid-header `Error::Config` at build; and
`identifier`/`bin_name` wiring. Shared pagination/retry helpers are tested in
`src/backends/mod.rs`.

## Related

- `release-tag-url-encoding.md` - tag percent-encoding on the by-tag route.
- `release-scan-pagination.md` - the shared `Link: rel="next"` pagination and page bound.
- `transport-control.md` - per-request headers, timeout, retries, client override.
- `error-network-vs-http-semantics.md` - non-2xx -> structured status variant vs parse -> `Error::Release`.
- `ref-release-model.md` - the `Release` / `ReleaseAsset` model the JSON maps onto.
- `async-api.md` - `build_async` and the public `AsyncReleaseUpdate` surface.

# GitHub backend (reference)

Status: implemented

## Scope

The GitHub release backend, `crate::backends::github`. It provides two
builder-driven entry points: `ReleaseList` (list a repo's releases) and `Update`
(download and install a release). Both target GitHub's REST releases API, with
support for a custom base URL (GitHub Enterprise) and optional token auth. This
document describes the existing behavior verified against `src/backends/github.rs`,
with shared HTTP helpers in `src/backends/common.rs` and `src/backends/mod.rs`.

## Behavior

### Builders

`ReleaseList::configure()` returns a `ReleaseListBuilder` with all fields `None`
and a default `RequestConfig` (`github.rs:152`). Setters: `repo_owner` (required),
`repo_name` (required), `filter_target` (optional asset-target filter,
`github.rs:85`), `url` (custom base URL, `github.rs:93`), `auth_token`
(`github.rs:104`), plus the shared transport setters from
`request_config_setters!(request)` (`github.rs:109`). The builder-setter
doc-aliases were dropped, so `filter_target`, `url`, and the others carry no
`#[doc(alias)]`. The string setters (`repo_owner`, `repo_name`, `url`,
`filter_target`, `auth_token`, and the `Update` builder's common setters) take
`impl Into<String>`. `build()` calls `self.request.check()` first (surfacing a
deferred `request_header` conversion error as `Error::InvalidHeader`), then requires
`repo_owner` and `repo_name`, bailing `Error::MissingField { field }` if either is missing, with
a message that names the setter to call (e.g. "`repo_owner` required (call
`.repo_owner(...)`)") (`github.rs:112-136`).

`Update::configure()` returns an `UpdateBuilder` (`Default`-constructed,
`github.rs:201`, `284`). Setters: `repo_owner` (required), `repo_name` (required),
`url` (custom base URL, no doc-alias, `github.rs:220`), plus the full shared
common surface from `impl_common_builder_setters!()` (`github.rs:225`): target,
bin name, current version, auth token, timeout, retries, request headers,
progress, confirm, asset matcher, verify hook, checksum, injected HTTP clients,
etc. `build_update()` requires `repo_owner`/`repo_name` (else `Error::MissingField { field }`,
with a message that names the setter, e.g. "`repo_owner` required (call
`.repo_owner(...)`)") and calls `self.common.build()` (which validates and
requires `current_version`/`bin_name`/`bin_path_in_archive`)
(`github.rs:227-248`). `build()` returns `Box<dyn ReleaseUpdate>`
(`github.rs:254`); `build_async()` (feature `async`) returns the concrete
`Update` so the inherent `*_async` methods are reachable (`github.rs:263`).

### Route shapes and base URL

The base URL is `self.custom_url`, defaulting to `https://api.github.com` when
unset. `ReleaseList` inlines this default (`github.rs:168-170`); `Update` centralizes
it in `api_base()` so the sync and async paths cannot drift (`github.rs:290-294`).
The `url(...)` doc comment specifies no trailing slash, e.g. `https://api.github.com`
or `https://github.mycorp.com/api/v3` (`github.rs:90-92`, `217-219`).

Routes built (all GET), each via a shared URL helper (`releases_url`, `latest_url`,
`tag_url`) so the sync and async paths cannot drift:
- List releases: `{base}/repos/{owner}/{name}/releases`. Used by `ReleaseList::fetch`,
  `get_latest_releases`, and `get_latest_releases_async`.
- Latest release: `{base}/repos/{owner}/{name}/releases/latest`. Used by
  `get_latest_release` / `get_latest_release_async`. This route returns a single release
  object (not an array).
- Fetch by tag: `{base}/repos/{owner}/{name}/releases/tags/{tag}` where `tag` is
  `urlencoding::encode(ver)`. Used by `get_release_version` and `get_release_version_async`.

The list routes go through `first_page_url` (`common.rs:58`), which appends
`?per_page=100` only when the base URL carries no `?` query.

### Auth

`auth_token` is optional and is carried as `Option<String>` on both `ReleaseList`
(its own field) and `Update` (via `CommonConfig::auth_token`). All requests build
headers via `api_headers(auth_token)` (`github.rs:488-507`): it always sets
`User-Agent: rust/self-update`, and when a token is present sets
`Authorization: token {token}` (the GitHub legacy "token" scheme, not "Bearer").
A token that fails to parse as a header value is surfaced as `Error::InvalidAuthToken`
(`github.rs:502`). There is no `GITHUB_TOKEN` environment-variable interplay
in this backend; the token must be supplied explicitly via `auth_token(...)`. The
`impl_update_config_accessors!` override arm wires github's `api_headers` into the
download path so the same User-Agent and token scheme are used there too
(`github.rs:369-373`).

### Pagination

The listing is described transport-free as a `PageRequest<Release>` via `releases_plan(base,
auth, stop_at)`: its parser maps the JSON array via `release_array_page` (calling
`ReleaseDto::into_release` per element), follows GitHub's
`Link: rel="next"` (`next_link`) into the next `PageRequest`, and sets `Page::stop` on the first
release not strictly newer than `stop_at`. The sync `run_paginated` and async
`run_paginated_async` drivers (`backends/mod.rs`) walk the chain, reusing the shared
`send`/`send_async` + `retry` machinery. Pagination is bounded by `MAX_RELEASE_PAGES = 100`; when
more pages are still advertised at the cap, a warning is logged and the walk stops with what was
collected. Per-page size defaults to 100 (`per_page=100`), but a base/next URL that already has
query params is used verbatim, so an existing `page`/`per_page` is not clobbered. `get_latest_releases`
passes `stop_at = Some(current_version)` (early-stop on the first not-newer release), while
`ReleaseList::fetch` passes `stop_at = None` and walks all pages. The single-object routes
(`/latest`, `/tags/{tag}`) use `single_plan`, whose parser yields `next: None`.

### JSON to model

Each page is parsed by `release_array_page`, which calls `ReleaseDto::into_release` on
each element:
- `tag_name` (required, else `Error::MissingAssetField { field: "tag_name" }`).
- `created_at` (required) into `date`.
- `name` (optional, falls back to `tag_name`).
- `body` (optional `String`).
- `assets` (required array, else `Error::MissingAssetField { field: "assets" }`), each parsed
  via asset DTO parsing.
- `version` is `tag_name` with a single leading `v` stripped via
  `trim_start_matches('v')`.

Asset DTO parsing requires `url` (download URL, else `Error::MissingAssetField { field: "url" }`)
and `name` (else `Error::MissingAssetField { field: "name" }`).

### Ordering

Releases are returned in the order GitHub's API returns them, which is newest
first; no client-side re-sort is applied. `ReleaseList::fetch` returns them as-is
(after the optional target filter) (`github.rs:175-181`). `Update`'s list paths
filter to strictly-newer-than-current via `bump_is_greater(current, r.version)`
but preserve order (`github.rs:331-334`, `461-464`).

### Errors

A completed non-2xx response is mapped to a structured variant by status,
identically for both clients: 404 -> `Error::NotFound`, 401/403 ->
`Error::Unauthorized`, any other non-2xx -> `Error::HttpStatus`
(`status_to_error`, `errors.rs:254`); a request that cannot complete
(connection/TLS/timeout) is `Error::Transport` (see
`fetch_all_releases_errors_on_http_error_status`, `github.rs:803-825`). A 200 body
that is not a JSON array on a list route yields `Error::NoReleaseFound { target: None }`
(`github.rs:387`, `415`). Missing required JSON fields yield `Error::MissingAssetField { field }`
(see JSON-to-model). Transport timeouts and retries are governed by `RequestConfig`
through the shared `send` / `send_async` helpers (`common.rs:173`, `194`).

## Public surface

- `ReleaseList`, `ReleaseListBuilder` (public structs; builder is `#[must_use]`).
- `ReleaseList::configure`, `ReleaseList::fetch`.
- `ReleaseListBuilder` setters: `repo_owner`, `repo_name`, `filter_target`, `url`,
  `auth_token`, the `request_config_setters!` transport setters, `build`.
- `Update`, `UpdateBuilder` (`Update` is `#[non_exhaustive]`; `UpdateBuilder` is
  `#[non_exhaustive]`-free but `#[must_use]`).
- `Update::configure`; `UpdateBuilder::new`, `repo_owner`, `repo_name`, `url`, the
  `impl_common_builder_setters!` surface, `build`, `build_async` (feature `async`).
- `Update` implements `ReleaseUpdate` (`get_latest_release`, `get_latest_releases`,
  `get_release_version`) and, under feature `async`, the public sealed `AsyncReleaseUpdate`
  (`get_latest_release_async`, `get_latest_releases_async`, `get_release_version_async`, plus the
  default `update_async` / `update_extended_async`).

The concrete `Update` (`github.rs:275`) carries `#[non_exhaustive]`. The `url`
setters carry no `#[doc(alias)]` (all builder-setter doc-aliases were dropped).
The setter was renamed from `with_url` (and earlier `instance_url`) to `url` in
earlier work, but no alias remains.

## Invariants and regression checklist

- The fetch-by-tag route percent-encodes the caller-supplied tag at every site:
  `get_release_version` (`github.rs:357`) and `get_release_version_async`
  (`github.rs:475`) both use `urlencoding::encode(ver)`. A tag with a URL-special
  `+` must appear as `%2B` on the wire, never raw.
- Releases are returned newest-first (GitHub API order), with no client-side
  re-sort in this backend.
- Route shapes are exactly `/repos/{owner}/{name}/releases`,
  `/repos/{owner}/{name}/releases/latest`, and
  `/repos/{owner}/{name}/releases/tags/{tag}` against the resolved base URL.
- Base URL defaults to `https://api.github.com`; the `Update` sync and async paths
  share `api_base()` so they cannot diverge.
- Auth header is `Authorization: token {token}` (legacy scheme), User-Agent is
  always `rust/self-update`.
- List per-page size defaults to 100 and pagination follows `Link: rel="next"`,
  bounded at 100 pages.
- `version` strips exactly one leading `v` from `tag_name`.

## Tests

In `src/backends/github.rs` (`#[cfg(test)] mod tests`), driven by a loopback TCP
stub (no external network):
- `get_release_version_percent_encodes_the_tag_in_the_url` (`github.rs:884`):
  asserts `/releases/tags/v1.0.0%2Bbuild.5` on the wire and no raw `+`.
- `fetch_all_releases_follows_link_pagination` (`github.rs:771`) and the async
  `fetch_all_releases_async_follows_pagination` (`github.rs:567`): two-page accumulation.
- `fetch_all_releases_errors_on_http_error_status` (`github.rs:803`): a non-2xx is
  the structured status variant (`NotFound`/`Unauthorized`/`HttpStatus`).
- `fetch_all_releases_errors_when_body_is_not_an_array` (`github.rs:827`):
  `Error::NoReleaseFound { target: None }`.
- `get_latest_release_sync_wraps_single_object_into_one_element_releases`
  (`github.rs:692`) and `..._reports_not_available_when_newest_equals_current`
  (`github.rs:719`): the `/latest` single-object path.
- `get_latest_releases_sync_returns_releases_and_precheck` (`github.rs:648`):
  strictly-newer filtering and the returned `Releases` pre-check.
- `api_headers_override_uses_github_user_agent_and_token_scheme` (`github.rs:964`):
  User-Agent `rust/self-update` and `Authorization: token secret`.
- `release_list_applies_its_request_config` (`github.rs:1349`): `ReleaseList`
  transport setters (retries) flow through `fetch`.
- Transport/builder tests: timeout, retries, custom request header on the wire,
  injected reqwest/ureq/async clients, progress/verify/checksum/asset-matcher storage.

## Related

- `release-tag-url-encoding.md` (tag percent-encoding at fetch-by-tag sites)
- `release-scan-pagination.md` (Link-header pagination, per-page sizing, page cap)
- `transport-control.md` (timeout, retries, custom headers, injected clients)
- `ref-release-model.md` (the `Release` / `ReleaseAsset` model and version
  normalization)
- `error-network-vs-http-semantics.md` (non-2xx -> structured status variant; transport failure -> `Error::Transport`)
- `choose-latest-release-sort.md` (ordering and newest-release selection)

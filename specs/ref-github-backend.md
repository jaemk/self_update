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
and a default `RequestConfig`. Setters: `repo_owner` (required),
`repo_name` (required), `filter_target` (optional asset-target filter),
`api_base_url` (custom API base URL, `github.rs:107`), `auth_token`,
plus the shared transport setters from
`request_config_setters!(request)`. The builder-setter
doc-aliases were dropped, so `filter_target`, `api_base_url`, and the others carry no
`#[doc(alias)]`. The string setters (`repo_owner`, `repo_name`, `api_base_url`,
`filter_target`, `auth_token`, and the `Update` builder's common setters) take
`impl Into<String>`. `build()` calls `self.request.check()` first (surfacing a
deferred `request_header` conversion error as `Error::InvalidHeader`), then requires
`repo_owner` and `repo_name`, bailing `Error::MissingField { field }` if either is missing
(`github.rs:126`). `ReleaseList` has `fetch` and, under the `async` feature, `fetch_async`
(`github.rs:216`), both returning `Result<Releases>`.

`Update::configure()` returns an `UpdateBuilder` (`Default`-constructed).
Setters: `repo_owner` (required), `repo_name` (required),
`api_base_url` (custom API base URL, no doc-alias, `github.rs:276`), plus the full shared
common surface from `impl_common_builder_setters!()`: target,
bin name, current version, auth token, timeout, retries, request headers,
progress, confirm, asset matcher, verify hook, checksum, injected HTTP clients,
etc. `build_update()` requires `repo_owner`/`repo_name` (else
`Error::MissingField { field }`) and calls `self.common.build()` (which validates and
requires `current_version`/`bin_name`/`bin_path_in_archive`).
`build()` returns the concrete `Update` (`github.rs:320`), as does
`build_async()` (feature `async`, `github.rs:329`). `Update` is `Send` and exposes
the update verbs as inherent methods (`update`, `update_extended`,
`get_latest_release`, `get_newer_releases`, `get_release_version`,
`is_update_available`), so `.build()?.update()?` needs no trait import.
`is_update_available()` returns `Result<Option<Release>>`: the newest
strictly-newer release, or `None` when up to date.

### Route shapes and base URL

The base URL is `self.custom_url`, defaulting to `https://api.github.com` when
unset. `ReleaseList` inlines this default; `Update` centralizes
it in `api_base()` so the sync and async paths cannot drift.
The `api_base_url(...)` setter takes the full API base including any path prefix, with no
trailing slash, e.g. `https://api.github.com` or `https://github.mycorp.com/api/v3`.

Routes built (all GET), each via a shared URL helper (`releases_url`, `latest_url`,
`tag_url`) so the sync and async paths cannot drift:
- List releases: `{base}/repos/{owner}/{name}/releases`. Used by `ReleaseList::fetch` /
  `fetch_async`, `get_newer_releases`, and `get_newer_releases_async`.
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
A token that fails to parse as a header value is surfaced as `Error::InvalidAuthToken`.
There is no `GITHUB_TOKEN` environment-variable interplay
in this backend; the token must be supplied explicitly via `auth_token(...)`. The
`impl_update_config_accessors!` override arm wires github's `api_headers` into the
download path so the same User-Agent and token scheme are used there too.

The token is host-gated: it is applied to the release-listing and binary-download requests, but
only to requests whose host matches the configured API host (or an `allow_auth_host` entry),
over https. A server-supplied asset `download_url` or pagination `Link` pointing at a different
host does not receive the token; `dangerously_allow_non_https_auth_forwarding()` relaxes the
https requirement for a host-matched request. A user-set `Authorization` via `request_header`
overrides the crate's token header.

### Pagination

The listing is described transport-free as a `PageRequest<Release>` via `releases_plan(base,
auth, stop_at)`: its parser maps the JSON array via `release_array_page` (calling
`ReleaseDto::into_release` per element) and follows GitHub's
`Link: rel="next"` (`next_link`) into the next `PageRequest`. The sync `run_paginated` and async
`run_paginated_async` drivers (`backends/mod.rs`) walk the chain, reusing the shared
`send`/`send_async` + `retry` machinery. Pagination is bounded by `MAX_RELEASE_PAGES = 100`; when
more pages are still advertised at the cap, a warning is logged and the walk stops with what was
collected. Per-page size defaults to 100 (`per_page=100`), but a base/next URL that already has
query params is used verbatim, so an existing `page`/`per_page` is not clobbered.
`get_newer_releases` passes `stop_at = Some(current_version)`, which filters per-item: each
release not strictly newer than the current version is dropped, but pagination continues through
all pages (an out-of-order listing cannot hide newer releases behind an old one).
`ReleaseList::fetch` passes `stop_at = None` and walks all pages unfiltered. The single-object
routes (`/latest`, `/tags/{tag}`) use `single_plan`, whose parser yields `next: None`.

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
(`status_to_error`); a request that cannot complete
(connection/TLS/timeout) is `Error::Transport` (see
`fetch_all_releases_errors_on_http_error_status`). A 200 body
that is not a JSON array on a list route yields `Error::InvalidResponse { source }` (the
serde_json parse error is chained via `source()`); an empty listing is the clean
`Error::NoReleaseFound { target: None }`. Missing required JSON fields yield
`Error::MissingAssetField { field }`
(see JSON-to-model). Transport timeouts and retries are governed by `RequestConfig`
through the shared `send` / `send_async` helpers.

## Public surface

- `ReleaseList`, `ReleaseListBuilder` (public structs; builder is `#[must_use]`).
- `ReleaseList::configure`, `ReleaseList::fetch`, `ReleaseList::fetch_async` (feature `async`).
- `ReleaseListBuilder` setters: `repo_owner`, `repo_name`, `filter_target`, `api_base_url`,
  `auth_token`, the `request_config_setters!` transport setters, `build`.
- `Update`, `UpdateBuilder` (`Update` is `#[non_exhaustive]`; `UpdateBuilder` is
  `#[non_exhaustive]`-free but `#[must_use]`).
- `Update::configure`; `UpdateBuilder::new`, `repo_owner`, `repo_name`, `api_base_url`, the
  `impl_common_builder_setters!` surface, `build`, `build_async` (feature `async`). Both `build`
  and `build_async` return the concrete `Update`.
- `Update` is `Send`, exposes the inherent verbs (`update`, `update_extended`,
  `get_latest_release`, `get_newer_releases`, `get_release_version`, `is_update_available`), and
  implements `ReleaseUpdate` plus, under feature `async`, the public sealed `AsyncReleaseUpdate`
  (`get_latest_release_async`, `get_newer_releases_async`, `get_release_version_async`, plus the
  default `update_async` / `update_extended_async`).

The concrete `Update` carries `#[non_exhaustive]`. The `api_base_url`
setters carry no `#[doc(alias)]` (all builder-setter doc-aliases were dropped).
The setter was renamed `url` -> `api_base_url` (and earlier `with_url` / `instance_url` ->
`url`), but no alias remains.

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
- The token is only sent to the configured API host (or an `allow_auth_host` entry) over https;
  a cross-host asset `download_url` or pagination link does not receive it.
- List per-page size defaults to 100 and pagination follows `Link: rel="next"`,
  bounded at 100 pages.
- `version` strips exactly one leading `v` from `tag_name`.

## Tests

In `src/backends/github.rs` (`#[cfg(test)] mod tests`), driven by a loopback TCP
stub (no external network):
- `get_release_version_percent_encodes_the_tag_in_the_url`:
  asserts `/releases/tags/v1.0.0%2Bbuild.5` on the wire and no raw `+`.
- `fetch_all_releases_follows_link_pagination` and the async
  `fetch_all_releases_async_follows_pagination`: two-page accumulation.
- `fetch_all_releases_errors_on_http_error_status`: a non-2xx is
  the structured status variant (`NotFound`/`Unauthorized`/`HttpStatus`).
- `fetch_all_releases_errors_when_body_is_not_an_array`:
  `Error::InvalidResponse` with a chained `source()`.
- `get_latest_release_sync_wraps_single_object_into_one_element_releases`
  and `..._reports_not_available_when_newest_equals_current`: the `/latest` single-object path.
- `get_newer_releases_sync_returns_releases_and_precheck` (`github.rs:1107`):
  strictly-newer filtering and the returned `Releases` pre-check.
- `get_newer_releases_continues_past_non_newer_releases_and_fetches_page_two`
  (`github.rs:708`): per-item filtering keeps paginating.
- `release_list_fetch_async_returns_releases_and_into_vec_recovers_them`
  (`github.rs:881`): the async listing path.
- `api_headers_override_uses_github_user_agent_and_token_scheme`:
  User-Agent `rust/self-update` and `Authorization: token secret`.
- `release_list_applies_its_request_config`: `ReleaseList`
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

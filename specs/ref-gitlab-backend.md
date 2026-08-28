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

`ReleaseList` lists releases for a repo and returns `Releases`. It is configured via
`ReleaseList::configure()`, which seeds `host` to `https://gitlab.com`.
The builder (`ReleaseListBuilder`) exposes `host`,
`repo_owner`, `repo_name`, `filter_target`, `auth_token`, the shared
`request_config_setters!(request)` setters, and `build()`
(`src/backends/gitlab.rs:ReleaseListBuilder::build`). `build()` calls `self.request.check()` first (surfacing any deferred
`request_header` error as `Error::InvalidHeader`), then requires `repo_owner` and `repo_name`,
each bailing `Error::MissingField { field }` when unset.
`repo_owner`/`repo_name` are stored `Option<String>` on the builder and resolved to
`String` on `ReleaseList`. `ReleaseList` has `fetch` (`src/backends/gitlab.rs:ReleaseList::fetch`) and, under the
`async` feature, `fetch_async` (`src/backends/gitlab.rs:ReleaseList::fetch_async`), both returning `Result<Releases>`.

`filter_target` (`src/backends/gitlab.rs:ReleaseListBuilder::filter_target`) sets a target that drops whole releases lacking a
matching asset; it is the `ReleaseList` release filter. `fetch()` applies it via
`r.has_target_asset(target)` (`src/backends/gitlab.rs:ReleaseList::fetch`). This differs from `Update::target`,
which selects *which asset* to download.

`Update` is built via `Update::configure()` -> `UpdateBuilder`.
Backend-specific setters are `host`, `repo_owner`, `repo_name`; all common
options come from `impl_common_builder_setters!()`. `build()` returns the concrete
`Update` (`src/backends/gitlab.rs:UpdateBuilder::build`), as does `build_async()` under the `async` feature
(`src/backends/gitlab.rs:UpdateBuilder::build_async`). `Update` is `Send` and exposes the update verbs as inherent methods
(`update`, `update_extended`, `get_latest_release`, `get_newer_releases`,
`get_release_version`, `is_update_available`). Both build paths delegate to
`build_update()`, which requires
`repo_owner`/`repo_name` (each bailing `Error::MissingField { field }`)
and calls `self.common.build()` (which runs the
deferred-header `check` and validates `current_version`, `bin_name`, `bin_path_in_archive`).
`UpdateBuilder::default()` seeds `host` to `https://gitlab.com`.

### Route shapes, host, and project-path encoding

The list/latest/newer routes share one base, `Update::releases_url()` (`src/backends/gitlab.rs:Update::releases_url`),
and `ReleaseList::fetch` builds the same shape inline (`src/backends/gitlab.rs:ReleaseList::fetch`):

```
<host>/api/v4/projects/<owner>%2F<repo>/releases
```

The literal `%2F` separating owner and repo is hard-coded in the format string in both places;
only `repo_owner` is run through
`urlencoding::encode`, while `repo_name` is interpolated verbatim. Encoding `repo_owner`
matters for subgroup paths (e.g. `group/subgroup` becomes `group%2Fsubgroup`) so an
embedded `/` does not create an extra path segment.

Fetch-by-tag (`get_release_version` / `get_release_version_async`) appends the tag to the
releases base via the shared `tag_url` helper (`src/backends/gitlab.rs:Update::tag_url`), percent-encoding the tag,
called from `get_release_version` (`src/backends/gitlab.rs:Update::get_release_version`) and `get_release_version_async`
(`src/backends/gitlab.rs:Update::get_release_version_async`):

```
{releases_url}/{urlencoding::encode(tag)}
```

This route returns a single release *object* (not an array), parsed directly by
`ReleaseDto::into_release` (called from `release_array_page`).

Custom host: `host(impl Into<String>)` (`src/backends/gitlab.rs:ReleaseListBuilder::host` `ReleaseListBuilder`, `src/backends/gitlab.rs:UpdateBuilder::host`
`UpdateBuilder`) overrides `host`. The
setter was renamed `url` -> `host` (and earlier `instance_url`/`with_host` -> `url`); it
carries no `#[doc(alias)]`
(all builder-setter doc-aliases were dropped). Its doc states the instance host only (scheme
+ host, no trailing slash and no `/api/v4`): the crate appends the `/api/v4/...` path itself,
so callers pass e.g. `https://gitlab.example.com`.
The string setters
(`host`, `repo_owner`, `repo_name`, `filter_target`, `auth_token`, and the `Update` builder's
common setters) take `impl Into<String>`.

### Authentication

The free `api_headers()` (`src/backends/gitlab.rs:api_headers`) sets only
`User-Agent: rust-reqwest/self-update`; the `Authorization: Bearer <token>` header is applied
centrally by the shared `apply_auth` (`backends/common.rs`) on both the listing and download
paths. The token is host-gated: it is only attached to requests whose host matches the
configured instance host (or an `allow_auth_host` entry), over https
(`dangerously_allow_non_https_auth_forwarding()` relaxes the https requirement), and a
user-set `Authorization` via `request_header` overrides it. A token that cannot be parsed
into a header value yields `Error::InvalidAuthToken`. There is no `PRIVATE-TOKEN` header and no
implicit environment lookup: the token comes from the builder setter
(`ReleaseListBuilder::auth_token`) or the common `auth_token` setter for `Update`
(`self.common.auth_token`), or on request from `auth_token_from_env()` on either builder, which
reads **`GITLAB_TOKEN` only** (`AUTH_TOKEN_ENV_VARS`, `src/macros.rs:AUTH_TOKEN_ENV_VARS`) and is a no-op when it
is unset. `CI_JOB_TOKEN` was deliberately removed from the list (`src/backends/gitlab.rs:ReleaseListBuilder`, `src/backends/gitlab.rs:UpdateBuilder`): it is
exported in every GitLab CI job, but this backend sends `Authorization: Bearer`, not GitLab's
job-token mechanism (the `JOB-TOKEN` header or a `job_token` request parameter), and job tokens are
project-scoped -- so keeping it meant `auth_token_from_env()` was not the advertised no-op inside
CI (it could turn a working anonymous fetch against a public project into a 401/403). An explicit
`auth_token(..)` with a non-blank value always wins over `auth_token_from_env()`, whatever the call
order (a blank explicit token is the documented exception:
`auth_token("").auth_token_from_env()` still picks up the env token, but
`auth_token_from_env().auth_token("")` discards it -- see `ref-common-config.md`);
`has_auth_token()` reports whether either source set a token.

An env-sourced token is bound to whatever host `host(..)` is configured with (or the default
`gitlab.com`), which `build()` flags via `env_token_host_decision`
(`src/backends/gitlab.rs:ReleaseListBuilder::build` `ReleaseListBuilder`, `src/backends/gitlab.rs:UpdateBuilder::build_update` `Update`). It returns
`EnvTokenDecision::WarnedAndSent` (warns, but still attaches the token) when the resolved host is
neither gitlab's canonical `gitlab.com` (`CANONICAL_AUTH_HOST`, `src/backends/gitlab.rs:CANONICAL_AUTH_HOST`) nor an acknowledged
`allow_auth_host` entry -- unchanged from before; a host passed to `allow_auth_host(..)` is now
acknowledged (`host_is_acknowledged`, `backends/common.rs`) and yields `EnvTokenDecision::Sent`
with no warning. An explicitly-set token is never warned about either. Both builders' `Debug`
impls redact the token to `"<token>"`.

### Pagination and ordering

Listing paths (`ReleaseList::fetch` / `fetch_async`, `get_newer_releases`, and the async
`get_newer_releases_async`) build a transport-free `PageRequest<Release>` via
`releases_plan(base, auth, stop_at)` and drive it with the sans-io `run_paginated` /
`run_paginated_async` drivers (`backends/mod.rs`). The plan starts at `first_page_url(base_url)`
(which appends `?per_page=100` when the URL has no query string), parses each page via
`release_array_page` (calling `ReleaseDto::into_release`), and follows GitLab's `Link: rel="next"` (`next_link`).
`get_newer_releases` passes `stop_at = Some(current_version)`, which filters per-item (each
release not strictly newer than the current version is dropped) while pagination continues
through all pages; `ReleaseList::fetch` passes `stop_at = None` and walks all pages unfiltered.
Pagination is bounded by
`MAX_RELEASE_PAGES` (100) in the driver; a still-advertised next page past that bound logs a
warning and stops.

Single-newest ordering: `get_latest_release` / `get_latest_release_async` use `newest_plan`, which
fetches just the first page and takes `releases[0]`. Unlike GitHub, GitLab has no dedicated
`/releases/latest` endpoint, so "newest" relies on the list endpoint's default descending
(newest-first) order. An empty array yields `Error::NoReleaseFound { target: None }`; a non-array
payload yields `Error::InvalidResponse { source }` (the serde_json error is chained).

Newer-than filtering is folded into the plan: with `stop_at = Some(current_version)`, the parser
keeps releases where `bump_is_greater(current_version, version)` is true and drops the rest
per-item, preserving order; pagination continues through all pages regardless.

### JSON to model

`ReleaseDto::into_release` (`src/backends/gitlab.rs:ReleaseDto::into_release`, called from `release_array_page`) maps a release
object, deserialized via the private `ReleaseDto` (`src/backends/gitlab.rs:ReleaseDto`, field `tag_name` /
`created_at` / `name` / `description` / `_links` / `assets`): `tag_name` -> required `version`
via `strip_tag_prefix` (trims a leading `v` when no `tag_prefix` is configured), missing ->
`Error::MissingAssetField { field: "tag_name" }`; `created_at` -> required `date`, missing ->
`Error::MissingAssetField { field: "created_at" }`; `name` -> `name`, defaulting to the tag when
absent; `description` -> optional `body`. Assets are read from `assets.links` (the private
`AssetsDto`, `src/backends/gitlab.rs:AssetsDto`), not a bare `assets` array; a missing `links` array yields
`Error::MissingAssetField { field: "assets.links" }`. Each asset is parsed by `AssetDto::into_asset`
(`src/backends/gitlab.rs:AssetDto::into_asset`), which requires `url` (-> `download_url`) and `name`, each bailing
`Error::MissingAssetField { field }` when missing.

### Errors

A completed non-2xx response is rejected by `send` / `send_async` before any body is parsed
and mapped to a structured variant by status (`status_to_error`, `src/errors.rs:status_to_error`): 404 ->
`Error::NotFound` (e.g. an unknown tag), 401/403 -> `Error::Unauthorized`, any other non-2xx
-> `Error::HttpStatus` (e.g. a 500/503 listing failure). A request that cannot complete is
`Error::Transport`. An empty array on the latest path yields `Error::NoReleaseFound { target: None }`;
a non-array listing body yields `Error::InvalidResponse { source }`;
missing JSON fields (`tag_name`, `created_at`, `assets.links`, asset `url`/`name`) yield
`Error::MissingAssetField { field }`. Builder/config problems (missing `repo_owner`/`repo_name`)
surface as `Error::MissingField { field }`; a deferred bad `request_header` surfaces as
`Error::InvalidHeader`; an unparseable auth token surfaces as `Error::InvalidAuthToken`.

## Public surface

- `ReleaseList::configure() -> ReleaseListBuilder`; `ReleaseList::fetch() -> Result<Releases>` (a
  bare listing: `current_version()` is `None`; recover the `Vec<Release>` with `into_vec()`);
  `ReleaseList::fetch_async()` (feature `async`).
- `ReleaseListBuilder`: `host`, `repo_owner`, `repo_name`, `filter_target`, `auth_token`,
  the `request_config_setters!` setters, `build() -> Result<ReleaseList>`.
- `Update::configure() -> UpdateBuilder`.
- `UpdateBuilder`: `new`, `host`, `repo_owner`, `repo_name`, the common setters,
  `build() -> Result<Update>`, and (feature `async`)
  `build_async() -> Result<Update>`.
- `Update` is `#[non_exhaustive]` and `Send`, exposes the inherent verbs (`update`,
  `update_extended`, `get_latest_release`, `get_newer_releases`, `get_release_version`,
  `is_update_available`), implements `ReleaseUpdate` and, under `async`, the public sealed
  `AsyncReleaseUpdate` (the `*_async` fetch verbs including `get_newer_releases_async`, plus the
  default `update_async` / `update_extended_async`).
- `host` carries no `#[doc(alias)]`; all builder-setter doc-aliases were dropped.

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
- Newer-than filtering is per-item and preserves order; pagination walks all pages.
- Auth uses `Authorization: Bearer <token>` plus a fixed `User-Agent`; no `PRIVATE-TOKEN`
  header. The token is only sent to the configured instance host (or an
  `allow_auth_host` entry) over https.
- `auth_token_from_env()` reads `GITLAB_TOKEN` only (`AUTH_TOKEN_ENV_VARS`); `CI_JOB_TOKEN` is
  deliberately not read (it is project-scoped and incompatible with the `Bearer` scheme this
  backend sends). An explicit `auth_token(..)` always wins over `auth_token_from_env()`, whatever
  the call order. `has_auth_token()` reports whether either source set a token. An env-sourced
  token bound to a non-`gitlab.com` host (from a custom `host(..)`) logs a warning at `build()` and
  is still sent, unless that host was also passed to `allow_auth_host(..)`, in which case it is
  acknowledged and no warning fires.
- A 429 is always `Error::RateLimited`; a 403 is `RateLimited` when it carries a spent quota
  (`RateLimit-Remaining: 0`) or a usable `Retry-After`; a bare 403 with neither stays
  `Unauthorized`. `retry`/`retry_async` never spend budget retrying a `RateLimited` response.
- `Update` is `#[non_exhaustive]`.

## Tests

In-file tests (`gitlab.rs`, `mod tests`) use a loopback `TcpListener` stub (no external
network). Coverage: sync and async latest/newer/by-tag parsing and version trimming;
`Link: rel="next"` pagination accumulation across two pages; per-item newer-than filtering
across pages; `is_update_available` sync and async paths; empty-array (`NoReleaseFound`) and
non-array (`InvalidResponse`) error paths; missing `tag_name` and missing
`assets.links` parser guards; non-2xx (404 -> `NotFound`, 500/503 -> `HttpStatus`);
`releases_url_encodes_repo_owner_with_slash` asserting `%2F` (and absence of a literal
`group/subgroup`) in the captured request line; `host`/`filter_target` setter existence;
`api_headers` User-Agent wiring and the centrally-applied Bearer scheme; invalid-header
`Error::InvalidHeader` at build; `ReleaseList::fetch_async` returning a bare listing; and
`identifier`/`bin_name` wiring. Shared pagination/retry helpers are tested in
`src/backends/mod.rs`. `AUTH_TOKEN_ENV_VARS` is pinned to `["GITLAB_TOKEN"]` on both builders
(`src/backends/gitlab.rs:auth_token_env_vars_are_gitlab_token_only`), with a comment noting `CI_JOB_TOKEN` must not be reintroduced.

## Related

- `release-tag-url-encoding.md` - tag percent-encoding on the by-tag route.
- `release-scan-pagination.md` - the shared `Link: rel="next"` pagination and page bound.
- `transport-control.md` - per-request headers, timeout, retries, client override.
- `error-network-vs-http-semantics.md` - non-2xx -> structured status variant vs parse ->
  `Error::InvalidResponse` / `Error::MissingAssetField`.
- `ref-release-model.md` - the `Release` / `ReleaseAsset` model the JSON maps onto.
- `async-api.md` - `build_async` and the public `AsyncReleaseUpdate` surface.

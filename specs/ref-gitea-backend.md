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

- `ReleaseList::configure()` returns `ReleaseListBuilder` (`gitea.rs:274`). The
  builder holds `host`, `repo_owner`, `repo_name`, `target`, `auth_token`, `auth_token_from_env`,
  and a `RequestConfig` (`gitea.rs:94-103`). It is `#[derive(Clone)]` with a hand-written `Debug`
  (`gitea.rs:105-124`, so `auth_token` redacts) and `#[must_use]`.
  `build()` validates and returns a `ReleaseList` (`gitea.rs:211-259`).
- `Update::configure()` returns `UpdateBuilder` (`gitea.rs:491-493`). The builder
  holds `host`, `repo_owner`, `repo_name`, and a `CommonBuilderConfig`
  (`gitea.rs:348-353`); it is `#[derive(Clone, Debug, Default)]` and `#[must_use]`.
  `UpdateBuilder::new()` is `Default::default()` (`gitea.rs:357-359`). The common
  setters (target, bin_name, current_version, auth_token, request headers, etc.) come
  from `impl_common_builder_setters!()` (`gitea.rs:399`).

`UpdateBuilder` exposes two terminal methods:

- `build()` returns the concrete `Update` (`gitea.rs:465-467`).
- `build_async()` (feature `async`) also returns the concrete `Update`, with the
  inherent `*_async` methods reachable (`gitea.rs:475-477`). There is no separate async
  builder type.

`Update` is `Send` and exposes the update verbs as inherent methods (`update`,
`update_extended`, `get_latest_release`, `get_newer_releases`, `get_release_version`,
`is_update_available`), so no trait import is needed. Both terminal methods delegate
to the private `build_update()` helper (see below). `ReleaseList` has `fetch` (`gitea.rs:295-313`)
and, under `async`, `fetch_async` (`gitea.rs:317`), both returning `Result<Releases>`.

### Route shapes and host

- The base releases route is built by `Update::releases_url()` (`gitea.rs:496-503`)
  and, identically, inline in `ReleaseList::fetch` (`gitea.rs:296-301`):
  `<host>/api/v1/repos/<owner>/<repo>/releases`.
- Fetch-by-tag appends `/tags/{tag}` to that base via the shared `tag_url` helper
  (`gitea.rs:510-512`), which percent-encodes the tag with `urlencoding::encode(ver)`:
  `<base>/tags/{encoded_tag}`, called from `get_release_version` (`gitea.rs:542`, sync) and
  `get_release_version_async` (`gitea.rs:734`, async).
- The custom host is set with `host(impl Into<String>)` on both builders
  (`gitea.rs:143` `ReleaseListBuilder`, `gitea.rs:373` `UpdateBuilder`). The setter
  carries no `#[doc(alias)]` (all builder-setter doc-aliases were dropped); it was
  renamed `url` -> `host` (and earlier `instance_url` / `with_host` -> `url`), but no
  alias remains. Its doc states the instance host
  only (scheme + host, no trailing slash and no `/api/v1`): the crate appends the
  `/api/v1/...` path itself. Gitea has no
  canonical public host, so `host` is required: `build()` / `build_update()` return
  `Error::MissingField { field: "host" }` when it is unset
  (`gitea.rs:242` `ReleaseListBuilder`, inside `build_update` `gitea.rs:415-459` for `Update`).
  The string setters (`host`, `repo_owner`,
  `repo_name`, `filter_target`, `auth_token`, and the `Update` builder's common
  setters) take `impl Into<String>`.
- Unlike GitHub, Gitea has no dedicated `/releases/latest` endpoint, so "latest" is
  derived from the list endpoint via `newest_plan` (`gitea.rs:661-686`, see ordering below).

### Auth

- `auth_token` is set on `ReleaseListBuilder` via `auth_token(impl Into<String>)`
  (`gitea.rs:182-189`, calling the shared `set_explicit_auth_token`); on `UpdateBuilder` it comes
  through the common setters and is stored in `CommonConfig`.
- `auth_token_from_env()` on either builder resolves the token from `GITEA_TOKEN`
  (`AUTH_TOKEN_ENV_VARS`, `gitea.rs:194`/`400`); it is opt-in (nothing reads the environment
  without it) and a no-op when the variable is unset or empty. An explicit `auth_token(..)` with a
  non-blank value always wins over it, whatever the call order; `has_auth_token()` reports whether
  either source set a token (a blank explicit token counts as unset here too). A blank explicit
  token is the exception to order-independence: `auth_token("").auth_token_from_env()` still picks
  up the env token, but `auth_token_from_env().auth_token("")` discards it (see
  `auth-token-from-env.md` AUTH-1-3).
- `build()` calls `env_token_host_decision` (`common.rs`), passing `None` for the canonical host
  (`gitea.rs:229-237` `ReleaseListBuilder`, `:447-455` `Update`): gitea has no well-known public
  instance (it is always self-hosted), so there is no host to compare an env-sourced token
  against. Unless the configured host was explicitly re-affirmed -- either an `auth_token(..)`
  call instead of `auth_token_from_env()`, or the same host also passed to
  `allow_auth_host(..)` -- the decision comes back `EnvTokenDecision::Withheld`: the caller clears
  `request.auth_token`, so the request goes out anonymous, and `build()` still returns `Ok`. A
  `log::warn!` names the host and both remedies. This is a change from the original behavior,
  where the call passed `None` for symmetry and never warned or acted, silently binding an
  ambient `GITEA_TOKEN` to whatever host was configured -- see `auth-token-from-env.md` AUTH-1-8
  for the full rationale and the other three backends' (unchanged) warn-and-send behavior.
- Headers are built by the free function `api_headers(auth_token)`.
  It always sets `User-Agent: rust-reqwest/self-update`. Auth is
  applied centrally by `apply_auth` (`common.rs`), which renders the token as
  `Authorization: token <token>` (the Gitea `token` scheme, not `Bearer`). The token
  is host-gated: it is only attached to requests whose host matches the configured
  instance host (or an `allow_auth_host` entry), over https;
  `dangerously_allow_non_https_auth_forwarding()` relaxes the https requirement, and a
  user-set `Authorization` via `request_header` overrides it. A token
  that cannot parse into a header value surfaces as `Error::InvalidAuthToken`.
- The `Update`'s `UpdateConfig` accessor override wires this same `api_headers`
  via `impl_update_config_accessors!` (`gitea.rs:569`), so the trait default
  (which sets no User-Agent) is not used.

### Pagination and ordering

- Listing follows Gitea's `Link: rel="next"` pagination via the sans-io core: `releases_plan(base,
  auth, stop_at)` builds a `PageRequest<Release>` whose parser maps each page via
  `release_array_page` (calling `ReleaseDto::into_release` per element) and follows
  `next_link(headers)`, driven by `run_paginated` /
  `run_paginated_async` (`backends/mod.rs`) starting from `first_page_url(base)` (which appends
  `?per_page=100` when no query is present). Pagination is bounded by `MAX_RELEASE_PAGES` (100) in
  the driver.
- "Single newest" is `releases[0]` of the first page via `newest_plan`; the code relies on the
  list endpoint's default descending (newest-first) order rather than sorting. The latest path does
  not paginate; it reads only the first response.
- The newer-releases paths (`get_newer_releases` / `get_newer_releases_async`) fold the
  strictly-newer filter into the plan: with `stop_at = Some(current_version)`, the parser keeps
  releases where `bump_is_greater(current, version)` is true and drops the rest per-item,
  preserving source order; pagination continues through all pages regardless.
  `ReleaseList::fetch` passes `stop_at = None` and walks all pages unfiltered.

### JSON to model

- Each page is parsed by `release_array_page`, which calls `ReleaseDto::into_release` on
  each element:
  - `tag_name` (required, else `Error::MissingAssetField { field: "tag_name" }`) -> `version`
    with a single leading `v` stripped via `trim_start_matches('v')`.
  - `created_at` (required, else `Error::MissingAssetField { field: "created_at" }`) -> `date`.
  - `name` -> `name`, defaulting to the tag when absent.
  - `assets` (required array, else `Error::MissingAssetField { field: "assets" }`) -> each
    mapped via asset DTO parsing.
  - `body` -> optional `body` (`None` when absent or non-string).
  - `browser_download_url` and `name` on each asset are required; either missing is
    `Error::MissingAssetField { field }`.
- `get_release_version[_async]` parses the bare object returned by `/tags/{tag}`
  directly (not wrapped in an array) (`gitea.rs:542`, `gitea.rs:734`), while the list
  endpoints parse a JSON array.

### Errors

- Missing host, owner, or name at build time -> `Error::MissingField { field }` with
  `field` naming the missing setter (`"host"`, `"repo_owner"`, `"repo_name"`).
- A deferred `request_header` conversion failure surfaces from `build()` via
  `request.check()` / `CommonBuilderConfig::build` as `Error::InvalidHeader`.
- An empty releases array -> `Error::NoReleaseFound { target: None }`; a non-array list
  payload -> `Error::InvalidResponse { source }` (the serde_json error is chained).
  A missing required JSON field ->
  `Error::MissingAssetField { field }` (see JSON-to-model above).
- A token that cannot parse into a header value -> `Error::InvalidAuthToken` (via `apply_auth`).
- Transport/HTTP failures propagate from the shared `send` / `send_async` helpers.

### `build_update` helper

`UpdateBuilder::build_update` (`gitea.rs:415-459`) is the private validator shared by
`build` and `build_async`. It resolves `host` / `repo_owner` / `repo_name` (erroring
as above) and calls `self.common.build()?` to produce the `CommonConfig`, returning a
concrete `Update`. Keeping it private ensures the sync and async terminal methods
validate identically and cannot drift.

## Public surface

- `gitea::ReleaseList`, `gitea::ReleaseListBuilder`
  - `ReleaseList::configure() -> ReleaseListBuilder`
  - `ReleaseListBuilder`: `host`, `repo_owner`, `repo_name`, `filter_target`,
    `auth_token`, the `request_config_setters!` setters, `build`
  - `ReleaseList::fetch() -> Result<Releases>` (filters by `target` when set; returns a `Releases` whose
    `current_version()` is `None`, so recover the `Vec<Release>` with `into_vec()`);
    `ReleaseList::fetch_async()` (feature `async`)
- `gitea::Update` (`#[non_exhaustive]`), `gitea::UpdateBuilder`
  - `Update::configure() -> UpdateBuilder`
  - `UpdateBuilder`: `new`, `host`, `repo_owner`, `repo_name`, common setters,
    `build`, `build_async` (feature `async`); both return the concrete `Update`
  - `Update` is `Send`, exposes the inherent verbs (`update`, `update_extended`,
    `get_latest_release`, `get_newer_releases`, `get_release_version`,
    `is_update_available`), and implements `ReleaseUpdate` (sync) and the public sealed
    `AsyncReleaseUpdate` (feature `async`, including `get_newer_releases_async`)
- Free `api_headers` and the `releases_plan` / `newest_plan` / `single_plan` plan builders are
  private to the module.

`Update` is `#[non_exhaustive]` (`gitea.rs:482`) so its fields stay private and future
fields do not break downstream code; it is constructed only through the builder.

## Invariants and regression checklist

- Tag is percent-encoded in the fetch-by-tag route via `urlencoding::encode`
  (`gitea.rs:338`, `gitea.rs:484`).
- Base route shape is exactly `<host>/api/v1/repos/<owner>/<repo>/releases`, shared by
  sync, async, and `ReleaseList` paths via `releases_url()` (`gitea.rs:314-319`).
- "Latest" is `releases[0]` of the first page, depending on the endpoint's newest-first
  ordering; the latest path does not paginate.
- Newer-release filtering is strict (`bump_is_greater`), folded into the page parser via
  `stop_at` as a per-item filter, and preserves source order; pagination walks all pages.
- `host` is required (no default host); missing it is `Error::MissingField { field: "host" }`.
- Auth uses the `token <token>` scheme with a fixed `rust-reqwest/self-update`
  User-Agent, and the token is only sent to the configured instance host (or an
  `allow_auth_host` entry) over https.
- `auth_token_from_env()` reads `GITEA_TOKEN` only (`AUTH_TOKEN_ENV_VARS`). An explicit
  `auth_token(..)` with a non-blank value always wins over it, whatever the call order.
  `has_auth_token()` reports whether either source set a token (a blank explicit token counts as
  unset). A blank explicit token is not order-independent: `auth_token("").auth_token_from_env()`
  still picks up the env token, but `auth_token_from_env().auth_token("")` discards it. Gitea has no
  canonical host to compare an env-sourced token's host against, so `env_token_host_decision`
  **withholds** it (clears `request.auth_token`, request goes out anonymous, `build()` still
  returns `Ok`) unless the configured host is acknowledged by an `allow_auth_host(..)` entry; a
  `log::warn!` names the host and both remedies. This differs from github/gitlab/gitee, which
  have a canonical host and so warn-and-still-send instead of withholding.
- A 429 is always `Error::RateLimited`; a 403 is `RateLimited` when it carries a spent quota or a
  usable `Retry-After`; a bare 403 with neither stays `Unauthorized`. `retry`/`retry_async` never
  spend budget retrying a `RateLimited` response.
- `version` has a single leading `v` stripped; `name` defaults to the tag.

## Tests

In `src/backends/gitea.rs` `mod tests` (starting `gitea.rs:764`), backed by a loopback
`TcpListener` stub (no external network):

- Sync `ReleaseUpdate` fetch: one-element latest wrap, strictly-newer filtering
  (e.g. `get_newer_releases_sync_reports_no_update_when_up_to_date`, `gitea.rs:1457`),
  no-update-when-up-to-date, and single-vs-list agreement.
- Builder shape: `host`/`filter_target` exist on `ReleaseListBuilder`; `ReleaseList`
  and `Update` builds require `host`, `repo_owner`, `repo_name`; invalid header surfaces
  as `Error::InvalidHeader`; `releases_url` shape; identifier and `bin_name` wiring.
- `api_headers` override uses the Gitea User-Agent and `token` scheme.
- `ReleaseList::fetch_async` returns a bare listing without a current version.
- Async (feature `async`): latest parse, `Link` pagination across two pages,
  `/tags/{ver}` single-object parse, missing-`tag_name` error, newer-only filtering,
  empty-when-up-to-date, accumulate-then-filter across pages, empty-array error
  (`NoReleaseFound`), non-array-payload error (`InvalidResponse`).
- `AUTH_TOKEN_ENV_VARS` is pinned to `["GITEA_TOKEN"]` on both builders (`gitea.rs:802-808`), with
  a comment guarding against a copy-pasted `GITHUB_TOKEN`-style list arriving here by mistake.

## Related

- `release-tag-url-encoding.md` (percent-encoding of the fetch-by-tag route)
- `transport-control.md` (request headers, timeout, retries, client override)
- `ref-release-model.md` (the `Release` / `ReleaseAsset` model these map into)
- `release-scan-pagination.md` (the shared `Link: rel="next"` pagination)
- `choose-latest-release-sort.md` (newest-first ordering assumptions)

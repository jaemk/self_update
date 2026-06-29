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

- `ReleaseList::configure()` returns `ReleaseListBuilder` (`gitea.rs:168`). The
  builder holds `host`, `repo_owner`, `repo_name`, `target`, `auth_token`, and a
  `RequestConfig` (`gitea.rs:62-69`). It is `#[derive(Clone, Debug)]` and `#[must_use]`.
  `build()` validates and returns a `ReleaseList` (`gitea.rs:121-153`).
- `Update::configure()` returns `UpdateBuilder` (`gitea.rs:309-311`). The builder
  holds `host`, `repo_owner`, `repo_name`, and a `CommonBuilderConfig`
  (`gitea.rs:206-209`); it is `#[derive(Clone, Debug, Default)]` and `#[must_use]`.
  `UpdateBuilder::new()` is `Default::default()` (`gitea.rs:214-216`). The common
  setters (target, bin_name, current_version, auth_token, request headers, etc.) come
  from `impl_common_builder_setters!()` (`gitea.rs:242`).

`UpdateBuilder` exposes two terminal methods:

- `build()` returns `Box<dyn ReleaseUpdate>` for the sync API (`gitea.rs:279-281`).
- `build_async()` (feature `async`) returns the concrete `Update` so the inherent
  `*_async` methods are reachable (`gitea.rs:287-290`). This is the "AsyncUpdate"
  entry point; there is no separate async builder type.

Both delegate to the private `build_update()` helper (see below).

### Route shapes and host

- The base releases route is built by `Update::releases_url()` (`gitea.rs:314-319`)
  and, identically, inline in `ReleaseList::fetch` (`gitea.rs:182-185`):
  `<host>/api/v1/repos/<owner>/<repo>/releases`.
- Fetch-by-tag appends `/tags/{tag}` to that base, with the tag percent-encoded via
  `urlencoding::encode(ver)`: `<base>/tags/{encoded_tag}`
  (`gitea.rs:338` sync, `gitea.rs:484` async).
- The custom host is set with `url(impl Into<String>)` on both builders. The setter
  carries no `#[doc(alias)]` (all builder-setter doc-aliases were dropped); it was
  renamed from `instance_url` / `with_host` in earlier work, but no alias remains.
  The setter's parameter is named `url` (matching the github/gitlab `url(url)` signature;
  behavior unchanged: it still writes `self.host`). Its doc states the instance host
  only (scheme + host, no trailing slash and no `/api/v1`): the crate appends the
  `/api/v1/...` path itself (`gitea.rs:71-77`, `gitea.rs:218-224`). Gitea has no
  canonical public host, so `url` is required: `build()` / `build_update()` `bail!`
  with `Error::Config` when it is unset, with a message that names the setter
  ("`url` required (gitea has no default host; call `.url(...)`)")
  (`gitea.rs:127-130`, `gitea.rs:250-253`). The string setters (`url`, `repo_owner`,
  `repo_name`, `filter_target`, `auth_token`, and the `Update` builder's common
  setters) take `impl Into<String>`.
- Unlike GitHub, Gitea has no dedicated `/releases/latest` endpoint, so "latest" is
  derived from the list endpoint (see ordering below) (`gitea.rs:340-343`).

### Auth

- `auth_token` is set on `ReleaseListBuilder` via `auth_token(impl Into<String>)`
  (`gitea.rs:113-116`); on `UpdateBuilder` it comes through the common setters and is
  stored in `CommonConfig`.
- Headers are built by the free function `api_headers(auth_token)`
  (`gitea.rs:531`). It always sets `User-Agent: rust-reqwest/self-update`. Auth is
  applied centrally by `apply_auth` (`common.rs`), which renders the token as
  `Authorization: token <token>` (the Gitea `token` scheme, not `Bearer`). A token
  that cannot parse into a header value surfaces as `Error::InvalidAuthToken`.
- The `Update`'s `UpdateConfig` accessor override wires this same `api_headers`
  via `impl_update_config_accessors!` (`gitea.rs:387-391`), so the trait default
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
- The newer-releases paths fold the strictly-newer filter into the plan: with `stop_at =
  Some(current_version)`, the parser keeps releases where `bump_is_greater(current, version)` is
  true and sets `Page::stop` at the first that is not (early-stop, relying on newest-first order),
  preserving source order. `ReleaseList::fetch` passes `stop_at = None` and walks all pages. The
  downstream `choose_latest_release` re-sort still selects the same release as a full walk.

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
  directly (not wrapped in an array) (`gitea.rs:383`, `gitea.rs:492`), while the list
  endpoints parse a JSON array.

### Errors

- Missing host, owner, or name at build time -> `Error::MissingField { field }`, with
  a message that names the missing setter (e.g. "`url` required (gitea has no default
  host; call `.url(...)`)") (`gitea.rs:127-146`, `gitea.rs:250-269`).
- A deferred `request_header` conversion failure surfaces from `build()` via
  `request.check()` / `CommonBuilderConfig::build` as `Error::InvalidHeader`
  (`gitea.rs:122`, `gitea.rs:271`).
- A non-array list payload or empty releases array -> `Error::NoReleaseFound { target: None }`
  (`gitea.rs:334-339`, `gitea.rs:456-461`). A missing required JSON field ->
  `Error::MissingAssetField { field }` (see JSON-to-model above).
- A token that cannot parse into a header value -> `Error::InvalidAuthToken` (via `apply_auth`).
- Transport/HTTP failures propagate from the shared `send` / `send_async` helpers.

### `build_update` helper

`UpdateBuilder::build_update` (`gitea.rs:245-273`) is the private validator shared by
`build` and `build_async`. It resolves `host` / `repo_owner` / `repo_name` (erroring
as above) and calls `self.common.build()?` to produce the `CommonConfig`, returning a
concrete `Update`. Keeping it private ensures the sync and async terminal methods
validate identically and cannot drift.

## Public surface

- `gitea::ReleaseList`, `gitea::ReleaseListBuilder`
  - `ReleaseList::configure() -> ReleaseListBuilder`
  - `ReleaseListBuilder`: `url`, `repo_owner`, `repo_name`, `filter_target`,
    `auth_token`, the `request_config_setters!` setters, `build`
  - `ReleaseList::fetch() -> Result<Releases>` (filters by `target` when set; returns a `Releases` whose
    `current_version()` is `None`, so recover the `Vec<Release>` with `into_vec()`)
- `gitea::Update` (`#[non_exhaustive]`), `gitea::UpdateBuilder`
  - `Update::configure() -> UpdateBuilder`
  - `UpdateBuilder`: `new`, `url`, `repo_owner`, `repo_name`, common setters,
    `build`, `build_async` (feature `async`)
  - `Update` implements `ReleaseUpdate` (sync) and the public sealed `AsyncReleaseUpdate`
    (feature `async`)
- Free `api_headers` and the `releases_plan` / `newest_plan` / `single_plan` plan builders are
  private to the module.

`Update` is `#[non_exhaustive]` (`gitea.rs:300`) so its fields stay private and future
fields do not break downstream code; it is constructed only through the builder.

## Invariants and regression checklist

- Tag is percent-encoded in the fetch-by-tag route via `urlencoding::encode`
  (`gitea.rs:338`, `gitea.rs:484`).
- Base route shape is exactly `<host>/api/v1/repos/<owner>/<repo>/releases`, shared by
  sync, async, and `ReleaseList` paths via `releases_url()` (`gitea.rs:314-319`).
- "Latest" is `releases[0]` of the first page, depending on the endpoint's newest-first
  ordering; the latest path does not paginate (`gitea.rs:343`, `gitea.rs:462`).
- Newer-release filtering is strict (`bump_is_greater`), folded into the page parser via
  `stop_at` / `Page::stop` (early-stop at the first non-newer release), and preserves source order.
- `url` is required (no default host); missing it is `Error::MissingField { field: "url" }`.
- Auth uses the `token <token>` scheme with a fixed `rust-reqwest/self-update`
  User-Agent.
- `version` has a single leading `v` stripped; `name` defaults to the tag.

## Tests

In `src/backends/gitea.rs` `mod tests` (`gitea.rs:517-1186`), backed by a loopback
`TcpListener` stub (no external network):

- Sync `ReleaseUpdate` fetch: one-element latest wrap, strictly-newer filtering,
  no-update-when-up-to-date, and single-vs-list agreement
  (`gitea.rs:628-754`).
- Builder shape: `url`/`filter_target` exist on `ReleaseListBuilder`; `ReleaseList`
  and `Update` builds require `url`, `repo_owner`, `repo_name`; invalid header surfaces
  as `Error::Config`; `releases_url` shape; identifier and `bin_name` wiring
  (`gitea.rs:756-976`).
- `api_headers` override uses the Gitea User-Agent and `token` scheme
  (`gitea.rs:779-809`).
- Async (feature `async`): latest parse, `Link` pagination across two pages,
  `/tags/{ver}` single-object parse, missing-`tag_name` error, newer-only filtering,
  empty-when-up-to-date, accumulate-then-filter across pages, empty-array error,
  non-array-payload error (`gitea.rs:978-1185`).

## Related

- `release-tag-url-encoding.md` (percent-encoding of the fetch-by-tag route)
- `transport-control.md` (request headers, timeout, retries, client override)
- `ref-release-model.md` (the `Release` / `ReleaseAsset` model these map into)
- `release-scan-pagination.md` (the shared `Link: rel="next"` pagination)
- `choose-latest-release-sort.md` (newest-first ordering assumptions)

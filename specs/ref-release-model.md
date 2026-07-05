# Release model and fetch traits (reference)

Status: implemented

## Scope

The release data model and the sealed fetch traits in `src/update.rs`: the
`Release` and `ReleaseAsset` value types and their asset-lookup helpers; the
`Releases` collection type and its query/ordering semantics; the sealed-trait
design (`ReleaseUpdate: UpdateConfig: sealed::Sealed`); and the exact contract
of each backend fetch method, including the `get_latest_release` vs
`get_newer_releases` distinction and async parity. The custom-backend
`ReleaseSource` / `AsyncReleaseSource` traits are covered only for the
fetch-method contract they document; the orchestration helpers
(`choose_latest_release`, `finish_update`, etc.) are out of scope.

## Behavior

### Release and ReleaseAsset

`ReleaseAsset` is a `#[non_exhaustive]` struct deriving `Clone, Debug, Default`
with two **encapsulated** (`pub(crate)`) fields, declared `name: Arc<str>` then
`download_url: Arc<str>`. The fields are backed by `Arc<str>` (not `String`) so
cloning a `ReleaseAsset` (and the `Release` that owns it) bumps a refcount rather
than reallocating the strings. Because it is `#[non_exhaustive]`, outside code
cannot build it with a struct literal; `ReleaseAsset::new(name, download_url)`
(taking `impl Into<String>`, converted to `Arc<str>`) is the public constructor.
The constructor argument order matches the field declaration order (`name`, then
`download_url`). The fields are read through getters that return borrows:
`name(&self) -> &str` and `download_url(&self) -> &str`.

`Release` is a `#[non_exhaustive]` struct deriving `Clone, Debug, Default` with
**encapsulated** (`pub(crate)`) fields `name: Arc<str>`, `version: Arc<str>`,
`date: Arc<str>`, `body: Option<Arc<str>>`, and `assets: Vec<ReleaseAsset>`
(again `Arc<str>`-backed for cheap clones). It is built from outside the crate
via `Release::builder()`, which returns a `ReleaseBuilder` (the builder stores
`String`s and converts to `Arc<str>` at `build()`); only `version` is required,
`name` defaults to the version, `date` defaults to empty, `body` to `None`. The
fields are read through getters returning borrows: `name(&self) -> &str`,
`version(&self) -> &str`, `date(&self) -> &str`, `body(&self) -> Option<&str>`,
and `assets(&self) -> &[ReleaseAsset]`. Callers (in-crate and downstream) read
releases exclusively through these getters; the in-crate construction/write sites
(the forge DTOs, the s3 parser) go through `Release::builder()` /
`ReleaseAsset::new` / the crate-private fields.

Asset lookup:

- `has_target_asset(target)`: `true` if any asset's `name` contains the `target`
  substring.
- `asset_for(target, identifier)`: returns the first matching `ReleaseAsset`
  (cloned), trying three tiers in order: (1) an asset whose name contains
  `target` and, if `identifier` is `Some`, also contains the identifier; (2)
  failing that, an asset whose name contains both the build OS
  (`std::env::consts::OS`) and ARCH (`std::env::consts::ARCH`) and the identifier
  if set; (3) failing that, and only when `identifier` is `Some`, an asset whose
  name contains the identifier. Returns `None` if no tier matches. Matching is
  plain substring (`str::contains`), not glob or regex.

### Releases

`Releases` is `#[non_exhaustive]`, derives `Debug, Clone`, and holds two private
fields: `releases: Vec<Release>` (ordered newest-first by the built-in backends)
and `current_version: Option<String>`. The current version is `Some` on the
updater path (where `is_update_available` is meaningful) and `None` for a bare
listing from `ReleaseList::fetch`, which has no version to compare against.

Constructors:

- `Releases::new(releases, current_version: String)` is `pub(crate)`: the updater
  path's constructor, storing `Some(current_version)`.
- `Releases::from_listing(releases)` is **public** (`update.rs:306`): the
  `ReleaseList::fetch` constructor, storing `None` for the current version. It
  also lets downstream tests build the bare-listing state.
- `Releases::from_releases(releases, current_version: impl Into<String>)` is the
  **public** test constructor (the type is `#[non_exhaustive]` with a crate-private
  primary constructor, so downstream code cannot build one with a struct literal).
  It stores `Some(current_version)`. The releases are taken as-is; no ordering is
  validated or imposed.

Accessors:

- `all(&self) -> &[Release]`: all releases as a slice, newest-first.
- `len(&self) -> usize`: number of releases held.
- `is_empty(&self) -> bool`: whether no releases are held.
- `current_version(&self) -> Option<&str>`: the configured current version the
  list was compared against, or `None` for a bare listing.
- `latest(&self) -> Option<&Release>`: the first element (`releases.first()`), or
  `None` when empty. This is the first element as ordered by the backend, not
  necessarily the semver maximum; a custom `ReleaseSource` may return an unsorted
  list.
- `into_vec(self) -> Vec<Release>`: consumes and returns the underlying vec, same
  order.
- `is_update_available(&self) -> Result<bool>`: `true` when **any** held release is
  strictly newer than the current version, via
  `version::bump_is_greater(current_version, r.version())`. The scan is
  order-independent (it examines the whole set, not just `latest()`), so it is
  correct for an unsorted custom list. It short-circuits on the first
  strictly-newer release, returning `Ok(true)` before later entries are examined;
  a found update therefore wins over a later parse error, and it is the first
  release *reached* whose version fails to parse that propagates its `Err`. An
  empty list yields `Ok(false)`. When no current version is known (a bare listing),
  it errors with `Error::MissingField { field: "current_version" }`. No further
  request is made; only already-fetched releases are consulted.

Iteration: owned `IntoIterator for Releases` (`:300-307`) yields `Release` by
value, consuming the collection (`std::vec::IntoIter`); borrowed
`IntoIterator for &'a Releases` (`:310-317`) yields `&'a Release` without
consuming (`std::slice::Iter`). Both iterate in `all()` order (newest-first).

### ReleaseStatus release accessors

`ReleaseStatus` (`#[non_exhaustive]`, `UpToDate` or `Updated(Release)`) carries
the installed `Release` on the `Updated` arm. Besides `into_version_status`,
`is_up_to_date`, and `is_updated`, it exposes three accessors that read the
installed release without forcing a `match` (which `#[non_exhaustive]` would
require a wildcard arm on): `updated_release(&self) -> Option<&Release>` borrows
it, `into_updated_release(self) -> Option<Release>` consumes the status and yields
it owned, and `version(&self) -> Option<&str>` returns the installed release's
version (mirroring `VersionStatus::version`, but `Some` only on `Updated`; the
`UpToDate` arm carries no version). All three return `None` for `UpToDate`.

### Sealed traits

The seal is `sealed::Sealed` (`src/update.rs:445-447`), a `pub(crate)` empty
trait implemented only inside the crate. `UpdateConfig: sealed::Sealed`
(`:462`) is the shared configuration/accessor surface (current version, target,
release tag, asset identifier, bin name/install path/path-in-archive, progress
and output flags, progress template/chars, auth token), plus the provided
`api_headers` helper. The crate-private plumbing accessors (request
timeout/headers/client, callbacks, matcher, checksum, keys) live on the
`pub(crate) trait UpdateInternals` (see `update-config-internal-accessors.md`).
`ReleaseUpdate: UpdateConfig` adds the fetch methods and
the provided `update` / `update_extended` flow. Because the supertrait chain
requires `sealed::Sealed`, neither trait can be implemented for a foreign type:
downstream code can *call* these traits but cannot *implement* them, leaving the
crate free to evolve the surface without a breaking change. Each backend
`build()` returns the concrete `Update` (not `Box<dyn ReleaseUpdate>`); the
`Update` is `Send` and exposes the verbs (`update`, `update_extended`,
`get_latest_release`, `get_newer_releases`, `get_release_version`, plus the
convenience `is_update_available() -> Result<Option<Release>>`, the newest
strictly-newer release or `None` when up to date) as inherent methods, so
`.build()?.update()?` needs no trait import.

The accessors live on `UpdateConfig` (the supertrait), not on `ReleaseUpdate`,
so they resolve on a `dyn ReleaseUpdate` value, on a generic `R: ReleaseUpdate`,
and on the narrower `U: UpdateConfig` bound used by the async orchestrator
(`update_extended_async`, `:851-857`) which needs the accessors but not the sync
fetch methods. The accessors borrow (e.g. `bin_install_path` returns `&Path`,
`current_version` returns `&str`), they do not return owned values.

`ReleaseSource` (`:350`) and `AsyncReleaseSource` (`:400`, `cfg(feature =
"async")`) are the custom-backend source traits and are **not** sealed: they
require `Send + Sync` and are meant to be implemented downstream. They are the
implementable counterpart to the sealed `ReleaseUpdate`.

### Fetch-method contracts

`ReleaseUpdate` exposes three sync fetch methods:

- `get_latest_release(&self) -> Result<Releases>`: a one-element
  `Releases` wrapping the **raw** newest release, unfiltered, carrying the
  configured current version. Because the newest release is always present,
  `latest()` is always `Some`, and `is_update_available()` returns `false` when
  that newest release is not strictly newer than the current version.
- `get_newer_releases(&self) -> Result<Releases>` (renamed from
  `get_latest_releases`): the candidate list
  as a `Releases`, newest-first, **filtered to releases strictly newer** than the
  configured current version. It is therefore empty (`latest()` is `None`) when
  already up to date, and any entry present is a genuine update. This is the
  documented distinction from `get_latest_release`: raw-newest vs
  strictly-newer-filtered.
- `get_release_version(&self, ver) -> Result<Release>`: the single
  `Release` matching an explicit tag/version (returns a bare `Release`, not a
  `Releases`).

The concrete backend `Update` types also expose the inherent
`is_update_available(&self) -> Result<Option<Release>>`, a convenience over
`get_newer_releases` returning the newest strictly-newer release (or `None` when
up to date).

The async counterparts are methods on the public sealed `AsyncReleaseUpdate` trait
(`cfg(feature = "async")`), used only through generics (never as a trait object) so its RPITIT
`async fn`s need no boxing: `get_latest_release_async() -> Result<Releases>`,
`get_newer_releases_async() -> Result<Releases>`, and
`get_release_version_async(ver) -> Result<Release>`. Each returns `impl Future<Output = ...> +
Send`, mirroring the sync method of the same name and the same raw-newest vs
strictly-newer-filtered distinction. The trait also carries default `update_async` /
`update_extended_async`; callers bring it into scope to call any verb.

The custom-source trait methods (`ReleaseSource`) take the same shape but return plain `Release` /
`Vec<Release>`: `get_latest_release()` is the single newest release; `get_latest_releases()` returns
the candidate list newest-first (the updater re-filters downstream, discarding non-newer entries,
preferring the newest semver-compatible one, and otherwise offering the newest available flagged
not-compatible, so the implementer need not filter and there is no `current_version` parameter);
`get_release_version(ver)` is the release for an explicit tag. `AsyncReleaseSource` mirrors these
with `impl Future<...> + Send` returns and the `Send` bound enforced at the impl site.

### Backend parsing and listing

The forge backends (github/gitlab/gitea) parse responses by deserializing the response bytes
directly into a private per-backend `#[derive(Deserialize)]` DTO (`ReleaseDto` / `AssetDto`, plus
gitlab's `AssetsDto` wrapping `assets.links`), then converting into the public `Release` /
`ReleaseAsset` via `Release::builder()` / `ReleaseAsset::new`. The DTOs are private, so
`Deserialize` is not part of the public `Release` / `ReleaseAsset` API. Each backend has its own DTO
because the JSON field names differ (github/gitea assets carry `url` / `browser_download_url`,
gitlab nests assets under `assets.links` and uses `description` for the body). s3 stays XML
(quick-xml streaming) and builds its `Release` / `ReleaseAsset` through the same constructors.

Each backend's `ReleaseList::fetch(&self) -> Result<Releases>` returns a `Releases` (matching the
updater's listing return type), built via `Releases::from_listing` with no current version;
`current_version()` is `None` and `is_update_available()` errors. Recover the raw `Vec<Release>`
with `into_vec()`.

## Public surface

- `pub struct ReleaseAsset` `#[non_exhaustive]` with `pub(crate)` `Arc<str>`
  fields `name`, `download_url`; `ReleaseAsset::new(name, download_url)`; getters
  `name() -> &str`, `download_url() -> &str`.
- `pub struct Release` `#[non_exhaustive]` with `pub(crate)` fields (`Arc<str>`
  `name`/`version`/`date`, `Option<Arc<str>>` `body`, `Vec<ReleaseAsset>`
  `assets`); `Release::builder()`, `has_target_asset`, `asset_for`; getters
  `name() -> &str`, `version() -> &str`, `date() -> &str`, `body() -> Option<&str>`,
  `assets() -> &[ReleaseAsset]`.
- `pub struct Releases` `#[non_exhaustive]`; `all`, `len`, `is_empty`,
  `current_version() -> Option<&str>`, `latest`, `into_vec`, `is_update_available`;
  owned and borrowed `IntoIterator`. `Releases::new` is
  `pub(crate)`; `Releases::from_releases(releases, current_version)` and
  `Releases::from_listing(releases)` are public.
- `ReleaseStatus::version() -> Option<&str>` (alongside `into_version_status`,
  `is_up_to_date`, `is_updated`, `updated_release`, `into_updated_release`).
- `pub trait UpdateConfig: sealed::Sealed` (accessors + `api_headers`).
- `pub trait ReleaseUpdate: UpdateConfig` (`get_latest_release`,
  `get_newer_releases`, `get_release_version`, `update`, `update_extended`).
- Each backend's concrete `Update` is `Send` and exposes the verbs plus
  `is_update_available() -> Result<Option<Release>>` as inherent methods.
- `pub trait ReleaseSource: Send + Sync` and (async) `AsyncReleaseSource` (not
  sealed). `pub trait AsyncReleaseUpdate: UpdateConfig` (async, sealed) and `pub(crate) mod sealed`.

## Invariants and regression checklist

- `ReleaseAsset` and `Release` and `Releases` stay `#[non_exhaustive]` with
  encapsulated (`pub(crate)`) fields; outside construction goes through
  `ReleaseAsset::new` / `Release::builder` / `Releases::from_releases` /
  `Releases::from_listing` (and the crate-internal `Releases::new`). Reads go through the
  getters (`name`/`version`/`date`/`body`/`assets`, `download_url`), which return
  borrows.
- `Release` / `ReleaseAsset` string fields are `Arc<str>`, so `Clone` shares the
  backing rather than reallocating; both stay `Clone + Debug + Default`.
- `Deserialize` is not part of the public `Release` / `ReleaseAsset` API; the forge
  backends parse through private per-backend DTOs.
- `ReleaseList::fetch` returns `Releases` (built via `from_listing`, no current
  version); `into_vec()` recovers the `Vec<Release>`.
- `asset_for` tier order is target+identifier, then OS+ARCH+identifier, then
  identifier-only; substring matching only.
- `Releases` is newest-first; `latest()` is `first()`, not the semver max.
- `is_update_available` scans the whole set (order-independent), short-circuits
  on the first newer release, returns `Ok(false)` on empty, and propagates the
  parse error of the first release reached that fails to parse.
- Owned and borrowed iteration both follow `all()` order.
- `get_latest_release` is raw newest (always `latest().is_some()`);
  `get_newer_releases` is strictly-newer-filtered (empty when up to date). Async
  siblings (`get_newer_releases_async`) preserve this. The custom-source
  `ReleaseSource::get_latest_releases` keeps its name (an unfiltered candidate
  list; the updater filters downstream).
- Accessors live on `UpdateConfig` and borrow; the trait chain stays sealed via
  `sealed::Sealed` so `ReleaseUpdate` / `UpdateConfig` cannot be implemented
  downstream, while `ReleaseSource` / `AsyncReleaseSource` remain implementable.

## Tests

In `src/update.rs` `mod tests` (`:977-1545`): `Releases` query/ordering
coverage (`releases_is_update_available_*` for newer-first, equal, empty,
newer-not-first, nothing-newer-unordered; `releases_latest_all_and_into_vec`,
`releases_latest_is_none_when_empty`, `releases_len_and_is_empty`,
`releases_current_version_accessor`); iteration order
(`releases_into_iterator_owned_in_order`, `..._borrowed_in_order`,
`..._empty_yields_nothing`, `..._order_matches_all`); sealed-trait bound-narrowing
compile locks (`accessor_via_release_update_bound`, `accessor_via_dyn_release_update`,
`accessor_via_update_config_bound`, exercised by `bound_narrowing_helpers_are_exercised`);
`bin_install_path_returns_a_borrow` pins the borrowing accessor; public async-by-tag
parity (`public_get_release_version_async_returns_tagged_release`,
`..._propagates_missing_tag_error`).

Encapsulation/`Arc<str>`/constructor coverage: `release_getters_return_builder_set_values` and
`release_builder_defaults_name_to_version_and_body_to_none` (getters surface the builder-set
values); `release_clone_shares_arc_backing` (a cloned `Release` shares the `Arc<str>` backing
pointer); `releases_from_releases_builds_a_usable_collection` and
`releases_from_listing_has_no_current_version_and_precheck_errors` (the public `from_releases`
constructor and the listing constructor's `None` current version);
`release_status_version_returns_installed_version_or_none` (`ReleaseStatus::version`). In
`src/backends/github.rs`: `release_list_fetch_returns_releases_and_into_vec_recovers_them`
(`ReleaseList::fetch -> Releases` plus `into_vec`) and
`github_dto_parses_sample_payload_through_getters` (a sample payload parsed through the private
DTO, asserted via the getters).

## Related

- `releases-check-type.md`
- `releases-test-constructor.md`
- `update-config-internal-accessors.md`
- `choose-latest-release-sort.md`
- `custom-backends.md`
- `custom-asset-matching.md`
- `async-api.md`

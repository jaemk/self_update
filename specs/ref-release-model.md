# Release model and fetch traits (reference)

Status: implemented

## Scope

The release data model and the sealed fetch traits in `src/update.rs`: the
`Release` and `ReleaseAsset` value types and their asset-lookup helpers; the
`Releases` collection type and its query/ordering semantics; the sealed-trait
design (`ReleaseUpdate: UpdateConfig: sealed::Sealed`); and the exact contract
of each backend fetch method, including the `get_latest_release` vs
`get_latest_releases` distinction and async parity. The custom-backend
`ReleaseSource` / `AsyncReleaseSource` traits are covered only for the
fetch-method contract they document; the orchestration helpers
(`choose_latest_release`, `finish_update`, etc.) are out of scope.

## Behavior

### Release and ReleaseAsset

`ReleaseAsset` (`src/update.rs:9-29`) is a `#[non_exhaustive]` struct deriving
`Clone, Debug, Default` with two public fields, declared `name: String` then
`download_url: String` (`src/update.rs:13-14`). Because it is `#[non_exhaustive]`,
outside code cannot build it with a struct literal; `ReleaseAsset::new(name,
download_url)` is the public constructor (`src/update.rs:23`). The constructor
argument order matches the field declaration order (`name`, then `download_url`).

`Release` (`src/update.rs:87-144`) is a `#[non_exhaustive]` struct deriving
`Clone, Debug, Default` with public fields `name: String`, `version: String`,
`date: String`, `body: Option<String>`, and `assets: Vec<ReleaseAsset>`. It is
built from outside the crate via `Release::builder()` (`src/update.rs:141`),
which returns a `ReleaseBuilder`; only `version` is required, `name` defaults to
the version, `date` defaults to empty, `body` to `None` (`src/update.rs:199-211`).

Asset lookup:

- `has_target_asset(target)` (`src/update.rs:100-102`): `true` if any asset's
  `name` contains the `target` substring.
- `asset_for(target, identifier)` (`src/update.rs:106-134`): returns the first
  matching `ReleaseAsset` (cloned), trying three tiers in order: (1) an asset
  whose name contains `target` and, if `identifier` is `Some`, also contains the
  identifier; (2) failing that, an asset whose name contains both the build OS
  (`std::env::consts::OS`) and ARCH (`std::env::consts::ARCH`) and the identifier
  if set; (3) failing that, and only when `identifier` is `Some`, an asset whose
  name contains the identifier. Returns `None` if no tier matches. Matching is
  plain substring (`str::contains`), not glob or regex.

### Releases

`Releases` (`src/update.rs:223-297`) is `#[non_exhaustive]`, derives
`Debug, Clone`, and holds two private fields: `releases: Vec<Release>` (ordered
newest-first by the built-in backends) and `current_version: String` (the
version the list was compared against). It is constructed by
`Releases::new(releases, current_version)` (`src/update.rs:233`), which is
`pub(crate)` and so not part of the public construction surface.

Accessors:

- `all(&self) -> &[Release]` (`:241`): all releases as a slice, newest-first.
- `len(&self) -> usize` (`:246`): number of releases held.
- `is_empty(&self) -> bool` (`:251`): whether no releases are held.
- `current_version(&self) -> &str` (`:256`): the configured current version the
  list was compared against.
- `latest(&self) -> Option<&Release>` (`:266`): the first element
  (`releases.first()`), or `None` when empty. This is the first element as
  ordered by the backend, not necessarily the semver maximum; a custom
  `ReleaseSource` may return an unsorted list.
- `into_vec(self) -> Vec<Release>` (`:271`): consumes and returns the underlying
  vec, same order.
- `is_update_available(&self) -> Result<bool>` (`:289-296`): `true` when **any**
  held release is strictly newer than `current_version`, via
  `version::bump_is_greater(current_version, r.version)`. The scan is
  order-independent (it examines the whole set, not just `latest()`), so it is
  correct for an unsorted custom list. It short-circuits on the first
  strictly-newer release, returning `Ok(true)` before later entries are examined;
  a found update therefore wins over a later parse error, and it is the first
  release *reached* whose version fails to parse that propagates its `Err`. An
  empty list yields `Ok(false)`. No further request is made; only already-fetched
  releases are consulted.

Iteration: owned `IntoIterator for Releases` (`:300-307`) yields `Release` by
value, consuming the collection (`std::vec::IntoIter`); borrowed
`IntoIterator for &'a Releases` (`:310-317`) yields `&'a Release` without
consuming (`std::slice::Iter`). Both iterate in `all()` order (newest-first).

### ReleaseStatus release accessors

`ReleaseStatus` (`src/update.rs:41`, `#[non_exhaustive]`, `UpToDate` or
`Updated(Release)`) carries the installed `Release` on the `Updated` arm. Besides
`into_version_status`, `is_up_to_date`, and `is_updated`, it exposes two accessors that read
the installed release without forcing a `match` (which `#[non_exhaustive]` would
require a wildcard arm on): `updated_release(&self) -> Option<&Release>`
(`src/update.rs:71`) borrows it, and `into_updated_release(self) -> Option<Release>`
(`src/update.rs:79`) consumes the status and yields it owned. Both return `None` for
`UpToDate`.

### Sealed traits

The seal is `sealed::Sealed` (`src/update.rs:445-447`), a `pub(crate)` empty
trait implemented only inside the crate. `UpdateConfig: sealed::Sealed`
(`:462`) is the shared configuration/accessor surface (current version, target,
release tag, asset identifier, bin name/install path/path-in-archive, progress
and output flags, progress template/chars, auth token, request timeout/headers/
client, progress and verify callbacks, asset matcher, and feature-gated
`verify_checksum` / `verify_keys`), plus the provided `api_headers` helper
(`:546-561`). `ReleaseUpdate: UpdateConfig` (`:578`) adds the fetch methods and
the provided `update` / `update_extended` flow. Because the supertrait chain
requires `sealed::Sealed`, neither trait can be implemented for a foreign type:
downstream code can *call* these traits (every backend `build()` returns a
`Box<dyn ReleaseUpdate>`) but cannot *implement* them, leaving the crate free to
evolve the surface without a breaking change.

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

- `get_latest_release(&self) -> Result<Releases>` (`:588`): a one-element
  `Releases` wrapping the **raw** newest release, unfiltered, carrying the
  configured current version. Because the newest release is always present,
  `latest()` is always `Some`, and `is_update_available()` returns `false` when
  that newest release is not strictly newer than the current version.
- `get_latest_releases(&self) -> Result<Releases>` (`:596`): the candidate list
  as a `Releases`, newest-first, **filtered to releases strictly newer** than the
  configured current version. It is therefore empty (`latest()` is `None`) when
  already up to date, and any entry present is a genuine update. This is the
  documented distinction from `get_latest_release`: raw-newest vs
  strictly-newer-filtered.
- `get_release_version(&self, ver) -> Result<Release>` (`:599`): the single
  `Release` matching an explicit tag/version (returns a bare `Release`, not a
  `Releases`).

The async counterparts are methods on the public sealed `AsyncReleaseUpdate` trait
(`cfg(feature = "async")`), used only through generics (never as a trait object) so its RPITIT
`async fn`s need no boxing: `get_latest_release_async() -> Result<Releases>`,
`get_latest_releases_async() -> Result<Releases>`, and
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

## Public surface

- `pub struct ReleaseAsset { pub name, pub download_url }` `#[non_exhaustive]`;
  `ReleaseAsset::new(name, download_url)`.
- `pub struct Release { pub name, version, date, body, assets }`
  `#[non_exhaustive]`; `Release::builder()`, `has_target_asset`, `asset_for`.
- `pub struct Releases` `#[non_exhaustive]`; `all`, `len`, `is_empty`,
  `current_version`, `latest`, `into_vec`, `is_update_available`; owned and
  borrowed `IntoIterator`. `Releases::new` is `pub(crate)`.
- `pub trait UpdateConfig: sealed::Sealed` (accessors + `api_headers`).
- `pub trait ReleaseUpdate: UpdateConfig` (`get_latest_release`,
  `get_latest_releases`, `get_release_version`, `update`, `update_extended`).
- `pub trait ReleaseSource: Send + Sync` and (async) `AsyncReleaseSource` (not
  sealed). `pub trait AsyncReleaseUpdate: UpdateConfig` (async, sealed) and `pub(crate) mod sealed`.

## Invariants and regression checklist

- `ReleaseAsset` and `Release` and `Releases` stay `#[non_exhaustive]`; outside
  construction goes through `ReleaseAsset::new` / `Release::builder` /
  (crate-internal) `Releases::new`.
- `asset_for` tier order is target+identifier, then OS+ARCH+identifier, then
  identifier-only; substring matching only.
- `Releases` is newest-first; `latest()` is `first()`, not the semver max.
- `is_update_available` scans the whole set (order-independent), short-circuits
  on the first newer release, returns `Ok(false)` on empty, and propagates the
  parse error of the first release reached that fails to parse.
- Owned and borrowed iteration both follow `all()` order.
- `get_latest_release` is raw newest (always `latest().is_some()`);
  `get_latest_releases` is strictly-newer-filtered (empty when up to date). Async
  siblings preserve this.
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

## Related

- `releases-check-type.md`
- `releases-test-constructor.md`
- `update-config-internal-accessors.md`
- `choose-latest-release-sort.md`
- `custom-backends.md`
- `custom-asset-matching.md`
- `async-api.md`

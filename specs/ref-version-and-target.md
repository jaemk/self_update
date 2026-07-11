# Version comparison and target detection (reference)

Status: implemented

## Scope

This spec documents two related mechanisms: (1) how the crate parses and compares
semver version strings (`src/version.rs`), and (2) how the running platform target
is determined and how a release asset is matched to it (`src/lib.rs`,
`src/update.rs`, `src/macros.rs`, `src/backends/common.rs`). It covers the default
asset matcher, the `target(...)` and `asset_matcher(...)` overrides, and the
tie-breaking/ordering used when multiple releases or assets qualify.

## Behavior

### Version parsing and comparison

All comparison helpers parse both operands with `semver::Version::parse` and operate
on the parsed `Version` (`src/version.rs:6`). They take bare `&str` version strings:

- `bump_is_greater(current, other)` returns `Ok(true)` iff `other > current` under
  full semver ordering, including prerelease and build-metadata rules
  (`src/version.rs:9-11`). This is the predicate that drives "is there an update".
- `bump_is_compatible(current, other)` (`src/version.rs:14-31`) encodes the crate's
  own compatibility policy (caret-like, with special-casing when `current` is a
  prerelease and when both majors are 0); a compatible bump must not itself be a
  prerelease except in the prerelease-current branch.
- `bump_is_major` / `bump_is_minor` / `bump_is_patch` compare the corresponding
  numeric fields (`src/version.rs:34-52`).
- `cmp_versions(a, b) -> Result<Ordering>` parses each version once and
  returns a true total order, including real `Equal` for equal versions (unlike the
  boolean `bump_is_greater`, which collapses equal/less into `false`). The shared
  release comparator `cmp_releases_newest_first(a, b) -> Ordering` builds on it for a
  newest-first order that places unparseable versions deterministically last (two
  unparseable compare `Equal`); it backs the selection sort in `choose_latest_release`
  and `s3::sort_newer` / `pick_latest`, so all three agree on "newest".

Parsing rules come entirely from the `semver` crate: strings must be `MAJOR.MINOR.PATCH`
with optional `-prerelease` and `+build` segments. Prerelease identifiers order below
the same core version; build metadata is ignored for ordering (standard semver).

No helper in `version.rs` strips a leading `v`. Prefix handling happens at the backend
boundary: the GitHub backend trims a single leading `v` from the release tag before it
becomes `Release.version` (`src/backends/github.rs:52`), so the value reaching these
helpers is already a bare semver string. A `Release` built via `Release::builder()` is
expected to carry a bare semver `version` (`src/update.rs:141-146`).

Error cases: any unparseable operand surfaces the underlying `semver::Error` converted
to `Error::SemVer` (boxed, opaque, source-preserving) via `From<semver::Error>`
(`src/errors.rs:53`, `src/errors.rs:154-157`). The helpers return `Result<bool>`, so a
parse failure propagates as `Err(Error::SemVer(_))` rather than a boolean.

Where parse errors are swallowed vs propagated:

- `Releases::is_update_available` calls `bump_is_greater` with `?` and short-circuits
  on the first strictly-newer release; a found update wins over a later parse error, but
  the first release *reached* with an unparseable version propagates `Error::SemVer`
  (`src/update.rs:267-274`).
- `choose_latest_release` is lenient: its filter and sort comparator use
  `bump_is_greater(...).unwrap_or(false)` / treat a comparator error as "not greater",
  so a release with an unparseable version is dropped rather than failing the update
  (`src/update.rs:642`, `src/update.rs:650-660`).

### Target string

`get_target()` returns the compile-time target triple as `&'static str` via
`env!("TARGET")` (`src/lib.rs:503-505`), e.g. `x86_64-unknown-linux-gnu` or
`i686-pc-windows-msvc`. It is the build target of the crate, captured at compile time
in `build.rs`; it is not recomputed at runtime.

The effective target used during an update is resolved at builder `build()` time:
`CommonBuilderConfig.target` is an `Option<String>` (`src/backends/common.rs:87`) that,
when unset, defaults to `get_target().to_owned()`, and when set is used verbatim
(`src/backends/common.rs:148-151`). The resolved `CommonConfig.target` is a plain
`String` (`src/backends/common.rs:191`) exposed through the `target()` accessor
(`src/macros.rs:129-131`).

### Asset matching and overrides

Asset selection happens in `resolve_and_confirm` (`src/update.rs:716-722`):

- If a custom `asset_matcher` is set, it is called with `&release.assets` and its
  `Option<ReleaseAsset>` result is used directly; the built-in `target`/`identifier`
  matching is bypassed entirely (`src/update.rs:718-719`).
- Otherwise `release.asset_for(target, asset_identifier())` runs the default matcher.
- Either way, `None` becomes `Error::NoReleaseFound { target: Some(..) }`
  (`resolve_and_confirm`, `src/update.rs`).

Default matcher `Release::asset_for` (`src/update.rs`) is substring-based and
tries three passes in order, returning the **first** matching asset (cloned):

1. First asset whose `name` contains `target` and (if set) `identifier`.
2. Else first asset whose `name` contains both the arch and os tokens derived from the
   configured `target` string by `target_arch_os(target)` (`src/update.rs`) - not the
   build host's `std::env::consts` values, so an explicitly configured cross-target
   selects its own assets - and (if set) `identifier`.
3. Else, only if `identifier` is set, the first asset whose `name` contains
   `identifier`.

`identifier` (set via `asset_identifier(...)`, `src/macros.rs:266`) disambiguates when
multiple assets match the same target; if unset, the first target match wins.
`has_target_asset` is the related `any(name.contains(target))` predicate the GitHub
backend uses to pre-filter releases (`src/update.rs:80-82`, `src/backends/github.rs:176`).

Overrides:

- `target(&str)` (`src/macros.rs:257-260`) overrides the platform string used by the
  default matcher and the `{target}` URL substitution.
- `asset_matcher(closure)` (`src/macros.rs:388-397`) installs an
  `Fn(&[ReleaseAsset]) -> Option<ReleaseAsset>` (boxed as `DynAssetMatcher`,
  `src/lib.rs:1136`) that fully replaces the default substring heuristic.

### Tie-breaking and ordering

- Multiple matching assets: the default matcher returns the **first** asset in list
  order that satisfies the earliest-succeeding pass; passes are ordered
  target+identifier, then OS+ARCH+identifier, then identifier-only.
- Multiple releases: `choose_latest_release` filters to strictly-newer releases,
  sorts them semver-descending (newest first) independent of source order, prefers the
  first *compatible* release, and falls back to the first (newest) release overall if
  none is compatible (`src/update.rs:640-692`).

## Public surface

- `self_update::get_target() -> &'static str` (`src/lib.rs:503`).
- `self_update::version::{bump_is_greater, bump_is_compatible, bump_is_major,
  bump_is_minor, bump_is_patch}(current, other) -> Result<bool>` (`src/version.rs`).
- `Release::has_target_asset(target)`, `Release::asset_for(target, identifier)`
  (`src/update.rs:80`, `src/update.rs:86`).
- Builder setters (each backend, via macro): `.target(&str)`,
  `.asset_identifier(&str)`, `.asset_matcher(closure)`
  (`src/macros.rs:257`, `:266`, `:388`).

## Invariants and regression checklist

- Comparison helpers never strip `v`; the `v`-trim lives in the GitHub backend
  (`src/backends/github.rs:52`). Backend `Release.version` must be bare semver.
- An unparseable version propagates as `Error::SemVer`, never a silent `false`, in
  `bump_is_greater`/`is_update_available`; but `choose_latest_release` deliberately
  drops unparseable releases via `unwrap_or(false)`.
- Unset `target` resolves to `get_target()`; a set `target` is used verbatim
  (`src/backends/common.rs:148-151`).
- A custom `asset_matcher` fully bypasses `asset_for`; no asset selected ->
  `Error::Release`.
- Default matcher pass order (target+id, OS+ARCH+id, id-only) and first-match
  semantics are stable.
- Release selection is order-independent: candidates are re-sorted descending before
  the compatible-first / newest-fallback choice.

## Tests

- `src/version.rs:54-114`: `test_bump_greater`, `test_bump_is_compatible`,
  `test_bump_is_major/minor/patch` cover the comparison matrix.
- `src/errors.rs:255-283`: `Error::SemVer` is opaque, source-preserving, and keeps the
  `SemVerError:` Display prefix.
- `src/backends/common.rs:318-336`:
  `build_resolves_target_and_install_path_defaults` pins unset-target-defaults-to-build
  -target and set-target-used-verbatim.
- `src/backends/github.rs:1059`: `asset_matcher_overrides_default_selection`.
- `src/update.rs:1181`: documents that an unparseable version is dropped by the leading
  `bump_is_greater(...).unwrap_or(false)` in `choose_latest_release`.

## Related

- `custom-asset-matching.md`
- `choose-latest-release-sort.md`
- `ref-release-model.md`
- `ref-update-pipeline.md`
- `error-variant-granularity.md`
- `ref-github-backend.md`

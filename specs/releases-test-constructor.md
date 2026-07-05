# Releases test constructor

Status: implemented

## Behavior

`Releases` is `#[non_exhaustive]` with a crate-private primary constructor
(`Releases::new` / `Releases::from_listing`), so downstream code cannot build one
with a struct literal. `Releases::from_releases(releases: Vec<Release>,
current_version: impl Into<String>) -> Self` is the public constructor, primarily
for building a `Releases` in downstream unit tests (e.g. a helper that takes a
`Releases` and inspects `latest()` / `is_update_available()`).

The releases are taken as-is; the built-in backends order them newest-first, but
no ordering is validated or imposed at construction. The supplied current version
is stored as the comparison version, so `current_version()` returns
`Some(current_version)` and `is_update_available()` compares against it (unlike the
listing constructor `from_listing`, which stores no current version and whose
`is_update_available()` errors).

## Public surface

- `Releases::from_releases(releases: Vec<Release>, current_version: impl
  Into<String>) -> Releases`.

## Tests

`src/update.rs` `mod tests`: `releases_from_releases_builds_a_usable_collection`
builds a `Releases` and asserts `latest()`, `current_version()`, and
`is_update_available()` work off it (both the update-available and up-to-date
cases).

## Related

- `ref-release-model.md`

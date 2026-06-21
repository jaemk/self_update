# Releases check type

Status: implemented

## Problem

A pre-check (is an update available?) used to fetch the release list twice: once via
the per-updater `is_update_available` and again at install time. The fetch surface
also diverged between the sync and async paths.

## Decision

A public `Releases` type carries the fetched releases (newest first) plus the
updater's current version. It is `#[non_exhaustive]`, derives `Debug` / `Clone`, and
is re-exported at the crate root (distinct from the per-backend `ReleaseList`
builder). The release-fetch surface on the sealed `ReleaseUpdate` trait returns it:
`get_latest_release()` and `get_latest_releases()` return `Result<Releases>` (the
latter no longer takes a `current_version` argument), while
`get_release_version(ver)` is unchanged. The per-updater `is_update_available` /
`is_update_available_async` checks were removed; the pre-check is now one fetch,
`updater.get_latest_releases()?.is_update_available()`.

Accessors and ergonomics:

- `all() -> &[Release]`, `latest() -> Option<&Release>` (newest), `into_vec(self) ->
  Vec<Release>`, `is_update_available() -> Result<bool>` (true when the newest release
  is strictly newer than the held current version, false when empty).
- `current_version()` accessor, `len()` / `is_empty()`, and `IntoIterator` for both
  the owned `Releases` and a `&Releases` borrow.

Async parity: under the `async` feature the built updater has
`get_latest_release_async()` and `get_latest_releases_async()` returning
`Result<Releases>`, and `get_release_version_async()` returning `Result<Release>`, so
the sync and async fetch surfaces match.

Note the contract distinction: `get_latest_release()` returns the raw newest release,
so its `is_update_available()` may be false (the newest release can equal or precede
the current version). `get_latest_releases()` returns the strictly-newer-filtered
list. Code wanting "is there a newer release" should use `get_latest_releases()`.

See `Releases` in `src/update.rs`, the crate-root re-export in `src/lib.rs`, and the
CHANGELOG `[unreleased]` Added and Changed entries.

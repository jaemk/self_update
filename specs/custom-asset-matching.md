# Custom asset matching (G7)

Status: implemented

## Problem

`Release::asset_for` used fixed substring matching (target-contains, then OS+ARCH,
then identifier-only). Releases with unconventional asset names, or selection rules
the substring heuristic could not express, could not be matched, so `update()` failed
with "no asset found".

## Decision

A matcher closure on every `Update` builder:
`asset_matcher(|assets: &[ReleaseAsset]| -> Option<ReleaseAsset>)`. When set it
overrides the built-in selection; returning `None` fails the update with "no asset
found". Default behavior is unchanged when unset. It is stored as
`AssetMatcher(Arc<DynAssetMatcher>)` (the same callback-newtype pattern used for the
progress callback, keeping the builder's `Clone` / `Debug` derives) and consumed in
`update_extended`. This refactor of the shared config also seeded the custom-backend
work.

See the `asset_matcher` setter in `src/macros.rs` / `src/update.rs` and the CHANGELOG
`[1.0.0]` Added entry.

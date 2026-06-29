# Async update API (G1)

Status: implemented

## Problem

Every entry point was blocking (`ReleaseList::fetch`, `Update::update` /
`update_extended`, `Download::download_to`). A tokio application had to wrap each
call in `spawn_blocking`, costing an extra thread per update with no integration
with the runtime's IO or cancellation.

## Decision

An additive `async` feature (tokio-only, reqwest-only; enabling it with `ureq` is
a `compile_error!`). Each built-in backend's `Update` builder gains `build_async()`
returning a concrete `Update` implementing the public sealed `AsyncReleaseUpdate`
trait, with the async verbs `update_async()`, `update_extended_async()`,
`get_latest_release_async()`, `get_latest_releases_async()`, and
`get_release_version_async()`. The blocking API is unchanged. Only the release
listing and the download are async; the response parsers are shared verbatim with
the sync path (no logic fork), and the verify/extract/install tail runs on
`tokio::task::spawn_blocking` so it does not block the executor.

`AsyncReleaseUpdate` (in `src/update.rs`) is the async counterpart of `ReleaseUpdate`,
sealed the same way (via `UpdateConfig: sealed::Sealed`). Its fetch verbs are
return-position `impl Trait` in trait (`impl Future<Output = ...> + Send`), and
`update_async` / `update_extended_async` are default methods routing to the free
`update::update_extended_async`. Being RPITIT it is not object-safe (nameable and
usable as a generic bound, like `AsyncReleaseSource`, never `dyn`). The custom
`AsyncReleaseSource` source trait is dispatched statically (no `async-trait`, no
boxed futures); `update_extended` is factored into shared helpers
(`choose_latest_release`, `resolve_and_confirm`, `build_download`,
`finish_update_owned`) that the async orchestrator reuses.

See the `async` feature in `Cargo.toml`, `src/http_client/`, `src/backends/`, and
the CHANGELOG.

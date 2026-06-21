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
returning a concrete `Update` with the async verbs `update_async()`,
`update_extended_async()`, and `get_latest_release_async()`. The blocking API is
unchanged. Only the release listing and the download are async; the response
parsers and the extract/install tail are shared verbatim with the sync path (no
logic fork). Internally an `AsyncReleaseSource` trait is dispatched statically (no
`async-trait`, no boxed futures); `update_extended` is factored into shared helpers
(`choose_latest_release`, `resolve_and_confirm`, `build_download`, `finish_update`)
that the async orchestrator reuses.

See the `async` feature in `Cargo.toml`, `src/http_client/`, `src/backends/`, and
the CHANGELOG `[unreleased]` Added entry. Design notes are in
`local/design-g1-async.md`.

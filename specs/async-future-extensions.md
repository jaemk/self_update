# Async future extensions

Status: not implemented

## Problem

The async API covers `update_async` / `update_extended_async` / the async fetch verbs
on the built-in backends, but a few async surfaces remain sync-only or inline.

## Implemented

- `spawn_blocking` for the extract and install tail: the verify/extract/replace tail
  runs on `tokio::task::spawn_blocking` so it does not block the executor. Delivered
  in 1.0 (see CHANGELOG).

## What it would take

- `ReleaseList::fetch_async` on each backend, so the standalone listing path has an
  async form like the update path does.
- Async custom backends beyond the current `AsyncReleaseSource`, for example exposing
  the async transport (`get_async`) to custom async sources, or a generic async
  pagination helper.

## Why deferred

Both remaining items are additive and non-blocking. The async listing path has no
demand yet.

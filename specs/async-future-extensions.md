# Async future extensions

Status: not implemented

## Problem

The async API covers `update_async` / `update_extended_async` / the async fetch verbs
on the built-in backends, but a few async surfaces remain sync-only or inline.

## What it would take

- `ReleaseList::fetch_async` on each backend, so the standalone listing path has an
  async form like the update path does.
- Optional `spawn_blocking` for the extract and install tail. It currently runs inline
  inside the async fn, briefly blocking the executor; moving it onto the blocking pool
  is cleaner for long-running servers but requires the data crossing the boundary to be
  `'static + Send`.
- Async custom backends beyond the current `AsyncReleaseSource`, for example exposing
  the async transport (`get_async`) to custom async sources, or a generic async
  pagination helper.

## Why deferred

All three are additive and non-blocking. The inline extract/install is fine for a
one-shot CLI update, and the async listing path has no demand yet.

# Custom backends (G4)

Status: implemented

## Problem

`ReleaseUpdate` is sealed, so downstream code could not support a release host the
built-in backends did not cover (another forge, a private registry, a plain HTTP
directory) without dropping to the low-level `Download` / `Extract` / `Move`
primitives and reimplementing the compare, confirm, download, verify, swap flow.

## Decision

A public, non-sealed `ReleaseSource` trait separates "where releases come from"
(user-owned) from "how the update happens" (crate-owned). It has the three sync
fetch methods plus `Send + Sync`. A `backends::custom::Update` holds a
`Box<dyn ReleaseSource>` and implements the still-sealed `ReleaseUpdate` by
delegating fetches, reusing the full orchestration. For natively-async sources, a
public `AsyncReleaseSource` trait (clean names, no `_async` suffix) drives a generic
`backends::custom::AsyncUpdate<S>` via `build_async()`; it is consumed through
generics, never a `dyn` object, so the futures need no `async-trait` or boxing. A
`backends::custom::Blocking` adapter wraps a `Clone` sync source to run on
`tokio::task::spawn_blocking`. `ReleaseAsset::new` and `Release::builder()`
(`ReleaseBuilder`) make those `#[non_exhaustive]` types constructible downstream.
`.retries()` is a documented no-op for the custom backend.

See `src/backends/custom.rs`, `examples/custom.rs`, and the CHANGELOG `[1.0.0]` Added
and Changed entries. Design notes are in `local/design-g4-custom-backends.md`.

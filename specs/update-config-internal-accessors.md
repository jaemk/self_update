# UpdateConfig internal accessors

Status: implemented (WS5 / B1)

## Resolution

The crate-private-typed accessors (`request_timeout`, `request_headers`, `request_config`,
`request_client`, `request_async_client`, `progress_callback`, `verify_callback`, `asset_matcher`,
and the feature-gated `verify_checksum` / `verify_keys`) now live on a separate `pub(crate) trait
UpdateInternals: sealed::Sealed` in `src/update.rs`. The public sealed `UpdateConfig` keeps only
public-typed accessors plus `api_headers`. `ReleaseUpdate` and `AsyncReleaseUpdate` require
`UpdateInternals` as a supertrait (so the orchestrator bounds `U: ReleaseUpdate` /
`U: AsyncReleaseUpdate` still reach the internal accessors), and `resolve_and_confirm`,
`build_download`, and `FinishCtx::capture` re-bound to `U: UpdateConfig + UpdateInternals`. The
`impl_update_config_accessors!` macro emits a separate `impl UpdateInternals` block per backend
(`@internals` arm), covering github/gitlab/gitea/s3/custom and the generic `AsyncUpdate<S>`.

## Problem

The sealed `UpdateConfig` supertrait carries `#[doc(hidden)]` accessor methods whose
signatures name crate-private types (for example `ClientOverride`, `DynProgressFn`,
and the other callback/transport newtypes). Even though the trait is sealed and the
methods are hidden, those signatures are technically part of the public trait
contract, so the crate-private types appear in the public API shape.

## What it would take

Move the internal-typed accessors onto a separate `pub(crate)` sub-trait that the
orchestration uses, leaving `UpdateConfig` with only the accessors whose signatures
name public types. The built-in backends and the custom backend implement both, and
`update_extended` / the async orchestrator read the internal accessors through the
`pub(crate)` trait. The research is in confirming this split does not disrupt the
orchestration (which currently bounds on `ReleaseUpdate` and reads every accessor) and
that the macro-generated accessor impls can be partitioned cleanly.

## Why deferred

Cosmetic on a sealed, `#[doc(hidden)]` surface. The crate-private types are not
namable or constructible downstream, so the leak is shape-only. Deferred pending the
research above.

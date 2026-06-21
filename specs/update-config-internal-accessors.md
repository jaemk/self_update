# UpdateConfig internal accessors

Status: needs research

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

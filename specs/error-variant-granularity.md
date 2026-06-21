# Error variant granularity

Status: partially implemented

## Problem

The catch-all stringly-typed variants `Error::Update(String)`,
`Error::Release(String)`, and `Error::Config(String)` carry only a message. A caller
cannot match on them to handle distinct failure modes (for example, distinguishing a
missing release from an unparseable response, or a config error from the field that
caused it).

## Shipped

The HTTP-status part of this is done. `Error::Network(String)` is gone, replaced by the
structured `Error::NotFound { url }`, `Error::Unauthorized { status, url }`, and
`Error::HttpStatus { status, url }`, with an `Error::http_status()` accessor, so a caller
can distinguish release-not-found from auth failure from other statuses without parsing a
message (see `ref-errors.md` and `error-network-vs-http-semantics.md`). The
`Update`/`Release`/`Config` string variants are unchanged and still the remaining gap.

## What it would take

Add finer-grained variants alongside (or in place of) the string variants, for
example a structured `NotFound`, `RateLimited { retry_after }`, `InvalidResponse`, or
config variants that name the offending field. The existing string variants can stay
as a fallback, or the call sites that build them can be repointed at the new variants.
Each new variant needs a `Display` arm and, where wrapping a source, a `source()` arm.

## Why deferred

`Error` is `#[non_exhaustive]`, so finer variants can be added later without a
breaking change. There is no demand signal yet for a specific split, and inventing a
taxonomy before a concrete need risks variants that do not match real handling
patterns. Deferred until a caller has a concrete fine-grained-handling need.

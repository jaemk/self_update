# choose_latest_release sort comparator

Status: needs research

## Problem

The descending sort comparator in `choose_latest_release` (`src/update.rs`) compares
two releases via `version::bump_is_greater(&y.version, &x.version)` and maps the
boolean to `Less` / `Greater`, never returning `Equal`. For two equal-version
releases it returns `Greater` for `(x, y)` and `Greater` for `(y, x)`, which is not
antisymmetric. The same shape exists in `backends::s3::sort_newer`. Today this is
harmless because `sort_by` is stable and the downstream selection only takes the
first compatible release, but it is a correctness smell: an unstable sort or a changed
selection could surface it.

## What it would take

A total-order comparator that parses both versions once and returns a proper
`Ordering` (including `Equal` for equal versions), with unparseable versions ordered
deterministically (for example sorted last) rather than via a boolean fallback. Share
it between `choose_latest_release` and `s3::sort_newer`. The research is in confirming
the version comparison can yield a true `Ordering` without changing which release is
selected in the existing tests, and in deciding the placement of unparseable versions.

## Why deferred

The current behavior is correct under the stable sort, so this is not a freeze
blocker. It is tracked so the comparator gets a proper total order before any change
to the sort or selection relies on it.

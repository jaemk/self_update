# Releases test constructor

Status: needs research

## Problem

`Releases` has a `pub(crate)` constructor and is `#[non_exhaustive]`, so downstream
code cannot build a `Releases` value to exercise its own code in unit tests (for
example a helper that takes a `Releases` and inspects `latest()` /
`is_update_available()`). `Release` and `ReleaseAsset` already gained constructors
(`ReleaseAsset::new`, `Release::builder()`), but `Releases` did not.

## What it would take

A `#[doc(hidden)]` test constructor or a small builder that takes a set of releases
plus a current version and produces a `Releases`. The research is in deciding the
shape (a single `#[doc(hidden)] pub fn` vs a builder) and whether the held current
version and newest-first ordering should be validated or assumed at construction.

## Why deferred

No demand signal yet, and the appendix in the gap tracking flagged it as
"consider ... if demand appears". Adding a constructor later is non-breaking, so it
can wait for a concrete request.

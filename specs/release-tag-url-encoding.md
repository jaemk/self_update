# Release-tag URL encoding

Status: not implemented

## Problem

The fetch-by-tag paths interpolate the caller-supplied tag directly into the request URL with no
percent-encoding. `github::get_release_version` (and its `_async` sibling) build
`.../releases/tags/{ver}`, and gitlab/gitea do the same with their tag routes. A tag containing
URL-special characters (`#`, `?`, a space) produces a malformed request rather than a clean
error. Tags are conventionally URL-safe, so this has not surfaced in practice, and the behavior
is unchanged from the pre-1.0 code.

## What it would take

Percent-encode the tag path segment before interpolation on all three git backends (sync and
async). That needs a percent-encoding routine available unconditionally: the `percent-encoding`
crate is currently pulled in only transitively under `s3-auth` (via `url`), so a fix would add a
new unconditional dependency, or a small hand-rolled encoder for the path-segment set.

## Why deferred

A new unconditional dependency (or a hand-rolled encoder) is not worth it for a rare,
conventionally-impossible input on the eve of a freeze. If a downstream report shows real tags
with special characters, encode the segment then. Until then the behavior matches every prior
release.

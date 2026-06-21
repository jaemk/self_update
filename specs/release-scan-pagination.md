# Release-scan pagination (G3)

Status: implemented

## Problem

`ReleaseList::fetch` paginated correctly, but `Update::update()` filtered compatible
versions via a per-backend `get_latest_releases` that issued a single request and
ignored pagination. For a repo with more than one page of releases, a newer
compatible release beyond the first page was silently missed and `update()` reported
"no update" when one existed.

## Decision

Factored the `Link: rel="next"` accumulation out of `ReleaseList::fetch` into a
shared bounded walk (`collect_paginated` / `next_link` / `first_page_url`) and called
it from both `ReleaseList` and the `update()` release scan, removing the duplication
that caused the drift. The s3 backend already paginated its listing.

See `src/backends/` (github/gitlab/gitea) and the CHANGELOG `[1.0.0]` Fixed entry.

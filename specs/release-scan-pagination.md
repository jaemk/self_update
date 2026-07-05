# Release-scan pagination (G3)

Status: implemented

## Problem

`ReleaseList::fetch` paginated correctly, but `Update::update()` filtered compatible
versions via a per-backend `get_latest_releases` that issued a single request and
ignored pagination. For a repo with more than one page of releases, a newer
compatible release beyond the first page was silently missed and `update()` reported
"no update" when one existed.

## Decision

Factored the `Link: rel="next"` accumulation into a shared bounded walk and called it from both
`ReleaseList::fetch` and the `update()` release scan, removing the duplication that caused the
drift. The s3 backend already paginated its listing.

## Sans-io core

The pagination walk is a transport-free description plus two thin drivers (`src/backends/mod.rs`):

- `PageRequest<T>` describes one page: the `url`, request `headers`, and a `parse: Box<dyn
  FnOnce(&[u8], &HeaderMap) -> Result<Page<T>> + Send>` that turns the body bytes and response
  headers into this page's items, the next `PageRequest` (if any), and an early-`stop` flag.
- `Page<T>` is `{ items, next: Option<PageRequest<T>>, stop: bool }`.
- `run_paginated(first, config)` (sync) and `run_paginated_async(first, config)` (async, feature
  `async`) loop: send the request through the existing `send`/`send_async` + `retry` machinery,
  read the body once, call `parse`, extend the accumulator, then stop on `page.stop`, on `next ==
  None`, or at the `MAX_RELEASE_PAGES` bound (logging the existing warning if a further page was
  still advertised at the bound).

Each built-in backend (github/gitlab/gitea/s3) writes its URL-building, JSON/XML parsing, and
version filtering ONCE as a transport-free plan builder plus a pure parser, with thin sync and
async wrappers that differ only by which driver they call. A single-shot fetch (github
`/releases/latest`, the `/releases/tags/{ver}` and gitlab/gitea per-tag endpoints, the
gitlab/gitea newest-from-first-page) returns a `PageRequest` whose parser yields `next: None`.

## Early stop (git release scan)

The git release-array parser sets `Page::stop = true` as soon as it sees a release NOT strictly
newer than the bound version (`bump_is_greater`), relying on the newest-first listing order. The
driver keeps the still-newer items already collected on that page and stops without fetching further
pages. The downstream `choose_latest_release` re-sort still selects the same release, so a full walk
and an early-stopped walk pick the identical release (selection parity). The bound is passed as
`stop_at`: the `update()` scan passes the current version, while `ReleaseList::fetch` is an
unfiltered listing and passes `stop_at = None` so it keeps walking ALL pages.

See `src/backends/` (github/gitlab/gitea/s3) and the CHANGELOG.

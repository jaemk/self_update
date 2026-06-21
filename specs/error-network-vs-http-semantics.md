# Error Network vs Http semantics

Status: implemented

## Problem

`Error::Network` was raised for a non-2xx HTTP response, while `Error::Http` held a
transport-level failure (a boxed `reqwest` / `ureq` error from the client). The naming
was counterintuitive: a reader expects "Http" to mean an HTTP status error and "Network"
to mean a connectivity failure, which is the reverse of how they were used. The two
clients also disagreed: a non-2xx surfaced as `Error::Network` on `reqwest` but as
`Error::Http` on `ureq`.

## What shipped

The 1.0 breaking window was used to restructure and rename rather than only re-document.
`Error::Http` is renamed to `Error::Transport` (a request that could not complete:
connection, TLS, timeout; the source error stays boxed). `Error::Network(String)` is
replaced by three structured variants for a completed non-2xx response:

- `Error::NotFound { url }` (HTTP 404)
- `Error::Unauthorized { status, url }` (HTTP 401/403)
- `Error::HttpStatus { status, url }` (any other non-2xx)

Both the `reqwest` and `ureq` clients now produce the same variants for the same status
(`src/http_client/reqwest.rs`, `src/http_client/ureq.rs`, via `errors::status_to_error`).
`Error::http_status() -> Option<u16>` returns the status for the three status variants and
`None` otherwise. The custom-backend `ReleaseSource` docs point implementors at the same
variants. See `ref-errors.md` for the full variant set and construction mapping, and
`error-variant-granularity.md` for the remaining stringly-typed variants.

## Why this resolves it

The names now match meaning (transport failure vs HTTP status), the two clients agree, and
a consumer can distinguish release-not-found from auth failure from other statuses by
matching the variant or calling `http_status()`, instead of string-parsing a message.

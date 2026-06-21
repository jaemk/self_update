# Transport control (G2)

Status: implemented

## Problem

The HTTP client was built internally per call. Callers could not set a timeout, a
proxy, retries, or arbitrary headers on the backend API or download requests, which
blocked corporate proxies, custom CA/mTLS setups, flaky networks, and per-request
gateway headers.

## Decision

Two phases, both additive.

Phase 1 added client-agnostic setters on the `Update` and `ReleaseList` builders:
`.timeout(Duration)`, `.request_header(name, value)`, and `.retries(n)` (retry with
exponential backoff). `Download::timeout(..)` covers the standalone downloader. Both
the reqwest and ureq clients honor the `HTTP(S)_PROXY` / `NO_PROXY` environment
variables.

Phase 2 added user-provided client injection: `reqwest_client`,
`reqwest_async_client` (under `async`), and `ureq_agent` on the builders and
`Download`. A client-agnostic `ClientOverride` carries the per-client field through
the request path, and the selected client crate is re-exported
(`self_update::reqwest` / `self_update::ureq`). The injected client is used for both
listing and download; `.request_header()` / `.retries()` (and reqwest `.timeout()`)
still apply, while proxy-env and the TLS feature defer to the injected client. One
client is reused across paginated requests.

See `src/http_client/`, `src/macros.rs` (`request_config_setters!`), and the
CHANGELOG `[unreleased]` and `[1.0.0]` Added entries. Design notes are in
`local/design-g2-phase2-client-injection.md`.

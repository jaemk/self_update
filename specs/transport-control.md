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

Phase 2 added user-provided client injection. The canonical seam is
`http_client(Arc<dyn HttpClient>)` (and `http_client_async(Arc<dyn
AsyncHttpClient>)` under `async`) on the builders and `Download`; the
client-specific `reqwest_client`, `reqwest_async_client`, and `ureq_agent` setters
are thin convenience wrappers that build a `ReqwestClient` / `ReqwestAsyncClient`
/ `UreqClient` and store it as the trait object. The injected
`Option<Arc<dyn HttpClient>>` is carried through `RequestConfig` and `Download`,
and the compiled client crate(s) are re-exported (`self_update::reqwest` /
`self_update::ureq`). The injected client is used for both listing and download;
`.request_header()` / `.retries()` (and, for a reqwest client, `.timeout()`)
still apply, while proxy-env and the TLS feature defer to the injected client.
One client is reused across paginated requests.

WS1 (self_update 3.0) replaced the compile-time-monomorphized transport with this
object-safe `HttpClient` trait seam: `reqwest` and `ureq` are no longer mutually
exclusive (both impls can compile, one is picked at runtime), the TLS features
coexist (rustls wins when both are on), and any `Arc<dyn HttpClient>` — including
a test double — can be injected.

See `src/http_client/`, `src/macros.rs` (`request_config_setters!`), and the
CHANGELOG `[unreleased]` and `[1.0.0]` Added entries. Design notes are in
`local/design-g2-phase2-client-injection.md`.

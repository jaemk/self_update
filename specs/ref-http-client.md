# HTTP client and transport (reference)

Status: implemented

## Scope

The client-agnostic HTTP layer the crate uses for every outbound GET: release
listing/lookup requests and the binary download. It covers the trait/dispatch
abstraction over `reqwest` and `ureq`, the request/response shape, header
handling via the re-exported `http` crate, mutually-exclusive client selection,
TLS selection, the transport setters (`timeout`, `request_header`, `retries`
with exponential backoff), proxy support, user-provided client injection and the
`ClientOverride` carrier, client reuse across paginated requests, and the
high-level Network-vs-Http error mapping.

## Behavior

### Abstraction and dispatch

There is no runtime trait-object dispatch across clients. Exactly one client
crate is compiled, and `http_client/mod.rs:11-14` re-exports either
`reqwest::*` or `ureq::*`, so `http_client::get` resolves to one concrete
function at compile time (`http_client/reqwest.rs:8`, `http_client/ureq.rs:9`).
Both `get` functions share the signature `(url, headers, timeout, client:
&ClientOverride) -> Result<impl HttpResponse>`.

Responses are abstracted by the `HttpResponse` trait
(`http_client/mod.rs:39-47`): `headers() -> &HeaderMap<HeaderValue>`, `body() ->
impl std::io::Read`, `json::<T>()`, `text()`. It is implemented for
`reqwest::blocking::Response` (`http_client/reqwest.rs:94-110`) and for ureq's
`Response<Body>` (`http_client/ureq.rs:60-76`). The async path is reqwest-only,
so it needs no trait: `AsyncResponse` is a type alias for `reqwest::Response`
(`http_client/mod.rs:20`) and `get_async` returns it directly
(`http_client/reqwest.rs:53-58`).

Headers use the `http` crate types throughout. `http_client/mod.rs:5-6`
re-exports `http::header` and `http::HeaderMap`; the whole `http` crate is
re-exported as `self_update::http` (`lib.rs:439`) so consumers can name header
types without a separate dependency.

### Client and TLS selection

`reqwest` and `ureq` are mutually exclusive. `reqwest` (plus `default-tls`) is
the default feature set; selecting `ureq` requires `default-features = false`
(`Cargo.toml:67,82-86`). Enabling both, or neither, is a hard
`compile_error!` (`lib.rs:414-421`). The `async` feature is reqwest-only and is
a `compile_error!` when combined with `ureq` (`lib.rs:433-436`).

TLS is chosen by feature, not at runtime. `default-tls` maps to each client's
native-TLS backend and `rustls` maps to each client's rustls backend
(`Cargo.toml:82-83`). For reqwest the rustls path calls `use_rustls_tls()` on
the per-call builder under `#[cfg(feature = "rustls")]`
(`http_client/reqwest.rs:28-31`, `73-76`); otherwise reqwest's default applies.
For ureq the per-call agent sets `TlsProvider::Rustls` under `rustls` and
`TlsProvider::NativeTls` otherwise (`http_client/ureq.rs:23-29`).

### Timeout, headers, retries and backoff

The shared setters are emitted by `request_config_setters!`
(`macros.rs:14-88`), writing into a `RequestConfig` (`backends/common.rs:29-40`)
that holds `timeout`, `headers`, `retries`, an injected `client`
(`ClientOverride`), and a deferred `header_error`.

- `timeout` sets a per-request timeout, default none, applied to every request
  the builder makes including the download (`macros.rs:18-21`).
- `request_header(name, value)` inserts one extra header; a repeated name
  overwrites. It is infallible at call time: an invalid name/value is stored as
  the first `header_error` (`backends/common.rs:46-72`) and surfaced from
  `build()` as `Error::Config` via `check()` (`backends/common.rs:75-80`).
- `retries` is the number of retries (default 0 = one attempt) for API requests
  only; the binary download is not retried, and it is a no-op on the custom
  backend (`macros.rs:41-54`).

The retry loop lives in `backends/mod.rs`, not in the http_client module.
`send` (`backends/mod.rs:173-189`) merges `config.headers` over the backend's
base headers, then calls `retry` with `http_client::get` as the attempt and a
closure that logs a warning and sleeps `backoff` ms between tries. `retry`
(`backends/mod.rs:117-135`) runs the attempt, and on error returns immediately
once `attempts >= retries`, otherwise sleeps `retry_backoff_ms(attempts)` and
increments. So `retries == 0` attempts exactly once; the budget boundary is
`>=`. Any failed attempt (including a permanent 404) consumes budget.

Backoff is `retry_backoff_ms(attempt) = 100u64 << attempt.min(5)`
(`backends/mod.rs:109-111`): 100, 200, 400, 800, 1600, 3200 ms, capped at
3200 ms for attempt 5 and beyond. The in-loop attempt index feeds the rising
backoff (not just index 0). `send_async` / `retry_async`
(`backends/mod.rs:141-211`) are the async siblings, using `tokio::time::sleep`;
the log runs synchronously between tries so the error is never held across the
await.

### Proxy

Both clients honor `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY`. reqwest does this
automatically. ureq's per-call agent sets `.proxy(ureq::Proxy::try_from_env())`
explicitly (`http_client/ureq.rs:31-32`). Proxy-from-env applies only to the
per-call client; an injected client is left to its own proxy config (see below).

### Client injection and ClientOverride

`ClientOverride` (`http_client/mod.rs:29-37`) is the client-agnostic carrier.
Only the field(s) for the compiled client exist: `blocking`
(`reqwest::blocking::Client`), `r#async` (`reqwest::Client`, under `async`),
`agent` (`ureq::Agent`). All are Arc-backed, so cloning shares the connection
pool. Setters: `reqwest_client`, `reqwest_async_client` (under `async`),
`ureq_agent` (`macros.rs:56-86`), each gated on its feature, plus matching
methods on `Download` (`lib.rs:1242-1261`). `set_client_override`
(`lib.rs:1265-1268`) forwards an `Update`'s injected client to its download.

When a field is set, the matching `get`/`get_async` reuses that client instead
of building one per call. The two clients are independent: a reqwest blocking
injection feeds the sync verbs and `reqwest_async_client` feeds the async ones;
injecting one and calling the other half falls back to a per-call client.

What still applies vs defers to the injected client:

- reqwest (`http_client/reqwest.rs:14-20`, `59-65`): the per-request `timeout`
  and `headers` are layered onto the injected client's request; TLS feature and
  proxy-env defer to the injected client.
- ureq (`http_client/ureq.rs:19-44`): the injected agent owns its own
  timeout/TLS/proxy, so the per-request `timeout` is applied only to the
  per-call agent and not to an injected agent; extra `request_header`s are still
  applied per request.
- `retries` is independent of the client: it wraps `get` in `send`, so an
  injected client is still retried.

### Reuse across paginated requests

`fetch_all_releases` walks `Link: rel="next"` pages via `collect_paginated`
(`backends/mod.rs:82-105`, `github.rs:368-385`), calling `send` once per page.
Each call passes `&config.client`, so an injected client (Arc-backed) is reused
across all pages, sharing its connection pool; a per-call client is rebuilt per
page. Pagination is bounded by `MAX_RELEASE_PAGES`. `collect_paginated_async`
(`backends/mod.rs:218-245`) is the async sibling.

### Error mapping (Network vs Http)

A transport-layer failure (connect/timeout/TLS) surfaces through the `?` on the
client's `send()` / `call()`, converted by `From<reqwest::Error>` /
`From<ureq::Error>` into `Error::Http` (`errors.rs:141-152`). A response with a
non-success status is mapped to `Error::Network` by the explicit status check in
each `get` (`http_client/reqwest.rs:37-44`, `82-89`,
`http_client/ureq.rs:48-55`). So "could not reach / talk to the server" is
`Http` and "reached the server, got a bad status" is `Network`.

## Public surface

- `self_update::http` (re-export of the `http` crate); `http_client::header`,
  `http_client::HeaderMap`.
- `self_update::reqwest` / `self_update::ureq` (re-export of the active client
  crate, `lib.rs:442-451`).
- Builder/`Download` setters: `timeout`, `request_header` / `header`,
  `retries`, `reqwest_client`, `reqwest_async_client`, `ureq_agent`.
- `HttpResponse` trait, `AsyncResponse` alias, `ClientOverride` (carrier;
  fields are `pub(crate)`).

## Invariants and regression checklist

- Exactly one HTTP client is compiled: both-or-neither is a `compile_error!`;
  `async` requires reqwest.
- TLS is feature-selected (`default-tls` native, `rustls`), never runtime.
- `retries == 0` means exactly one attempt; the exhaustion boundary is
  `attempts >= retries` (one retry => two attempts).
- Backoff sequence is 100/200/400/800/1600/3200 ms, capped at 3200 from attempt
  5 onward (`100 << attempt.min(5)`); the rising index is fed in-loop.
- The binary download is not retried (`Download` has no `retries`; it calls
  `http_client::get` directly, not `send`).
- An injected client still honors `request_header` and `retries`; for reqwest it
  also honors the per-request `timeout`, for ureq the timeout defers to the
  agent. Proxy-env and TLS defer to the injected client.
- Non-success status => `Error::Network`; transport failure => `Error::Http`.
- Injected clients are Arc-backed and reused across paginated pages.

## Tests

- `backends/mod.rs` retry/backoff unit tests: zero-budget attempts once,
  single-retry boundary, exponential-and-capped sequence, in-loop climb to cap,
  later-attempt success, async sibling (`backends/mod.rs:329-530`).
- `backends/github.rs`: `retries_recover_from_transient_failures`,
  `retries_are_exhausted_and_then_error`, `retries=1` boundary, timeout honored,
  pagination follows `Link` (`github.rs:1260-1364`, `764-833`).
- `backends/common.rs`: `insert_header` records invalid name/value, first-error
  wins, valid-then-invalid keeps the valid header (`common.rs:218-289`).
- `errors.rs`: boxed `source()` mirroring for `Http` and siblings.

## Related

- `transport-control.md`
- `error-network-vs-http-semantics.md`
- `ref-feature-flags.md`

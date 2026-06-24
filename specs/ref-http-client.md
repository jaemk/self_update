# HTTP client and transport (reference)

Status: implemented

## Scope

The client-agnostic HTTP layer the crate uses for every outbound GET: release
listing/lookup requests and the binary download. It covers the object-safe
trait/dispatch seam over `reqwest` and `ureq`, the request/response shape, header
handling via the re-exported `http` crate, non-exclusive client selection,
TLS selection (including coexistence), the transport setters (`timeout`,
`request_header`, `retries` with exponential backoff), proxy support,
user-provided client injection via `Arc<dyn HttpClient>`, client reuse across
paginated requests, and the high-level Network-vs-Http error mapping.

## Behavior

### Abstraction and dispatch

The transport is an **object-safe trait seam** dispatched at runtime, not a
compile-time monomorphized function. `http_client::HttpClient`
(`http_client/mod.rs`) has a single method `get(&self, url, headers, timeout) ->
Result<Box<dyn HttpResponse>>` (the crate only ever issues GETs), and each impl
maps a non-2xx status to the structured `NotFound`/`Unauthorized`/`HttpStatus`
variant *before* returning `Ok`. Retries are **not** in the trait — they stay in
`backends::send`/`retry`, wrapping `client.get(...)`.

Both client crates can be compiled at once: `ReqwestClient` (a
`reqwest::blocking::Client`, `http_client/reqwest.rs`) and `UreqClient` (a
`ureq::Agent`, `http_client/ureq.rs`) each `impl HttpClient`, namespaced so they
coexist. `default_client() -> Box<dyn HttpClient>` (`http_client/mod.rs`) selects
reqwest when the `reqwest` feature is on (preferred when both are enabled), else
ureq; a genuine no-client build is a `compile_error!`. `send`/`download_to` call
`config.client.as_deref().unwrap_or(&default).get(...)`.

Responses are abstracted by the `HttpResponse` trait (`http_client/mod.rs`):
`headers() -> &HeaderMap<HeaderValue>`, `json_value(&mut self) ->
serde_json::Value` (replacing the old generic `json::<T>()`, which made the trait
non-object-safe), `text(&mut self)`, `body(self: Box<Self>) -> Box<dyn Read>`,
and a `body_buffered` default wrapping `body()` in a `BufReader`. It is
implemented for `reqwest::blocking::Response` and ureq's `Response<Body>`.

The async path (reqwest + tokio only) has the sibling object-safe traits
`AsyncHttpClient`/`AsyncHttpResponse` (`http_client/mod.rs`). `AsyncHttpResponse`
exposes `headers()`, `text()`, and `bytes_stream() -> BoxStream<Result<Bytes>>`;
`download_to_async` drives `bytes_stream()` rather than leaking a concrete
`reqwest::Response`. `default_async_client()` is always reqwest. The `bytes`
crate is a direct optional dep gated under `async`.

Headers use the `http` crate types throughout. `http_client/mod.rs:5-6`
re-exports `http::header` and `http::HeaderMap`; the whole `http` crate is
re-exported as `self_update::http` (`lib.rs:439`) so consumers can name header
types without a separate dependency.

### Client and TLS selection

`reqwest` and `ureq` are **no longer mutually exclusive** — both impls can be
compiled and `default_client()` selects one at runtime (reqwest preferred when
both are on). `reqwest` (plus `rustls`) is still the default feature set; the
only hard requirement is at least one client (a no-client build is a
`compile_error!` in `http_client/mod.rs`). The `async` feature is reqwest-only;
because `async` already implies `reqwest` in `Cargo.toml`, `async` + `ureq`
together is fine (async drives the reqwest path, ureq serves the sync path). The
surviving guard only fires if `async` is somehow on without `reqwest`.

TLS is feature-selected, and the two TLS features **coexist**: when both
`native-tls` and `rustls` are enabled, the per-call builders prefer rustls (it is
the crate default). For reqwest the per-call builder calls `use_rustls_tls()`
under `#[cfg(feature = "rustls")]`, else `use_native_tls()` under
`#[cfg(all(feature = "native-tls", not(feature = "rustls")))]`, else reqwest's
default. For ureq the per-call agent sets `TlsProvider::Rustls` under `rustls`
and `TlsProvider::NativeTls` otherwise. This is what lets `cargo build
--all-features` (both clients + both TLS + async) build.

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

### Security invariants for `HttpClient` implementations

The `HttpClient` and `AsyncHttpClient` traits carry a documented security contract that custom
implementations must uphold:

1. TLS certificate verification must be enabled. Disabling it allows a man-in-the-middle to serve
   arbitrary binaries. The `danger_accept_invalid_certs` option (or equivalent) must never be set.

2. The `Authorization` header must not be forwarded to a different host on a redirect. An
   attacker-controlled redirect destination could harvest bearer tokens or API keys. The HTTP
   client must strip the `Authorization` header when following a redirect to a different origin.

Both built-in clients satisfy these: reqwest strips cross-host auth headers by default; the ureq
per-call agent sets `.redirect_auth_headers(RedirectAuthHeaders::Never)` explicitly
(`http_client/ureq.rs`). The ureq setting is explicit (not relying on the default) so a future
ureq version bump cannot silently change the policy. The contract is documented on the
`HttpClient` trait doc comment; `AsyncHttpClient` references it.

### Proxy

Both clients honor `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY`. reqwest does this
automatically. ureq's per-call agent sets `.proxy(ureq::Proxy::try_from_env())`
explicitly (`http_client/ureq.rs`). Proxy-from-env applies only to the
per-call client; an injected client is left to its own proxy config (see below).

### Client injection (Arc<dyn HttpClient>)

The canonical injection seam is `Option<Arc<dyn HttpClient>>` (and, under
`async`, `Option<Arc<dyn AsyncHttpClient>>`) on `RequestConfig`
(`backends/common.rs`) and on `Download` (`lib.rs`). The primary setters are
`http_client(Arc<dyn HttpClient>)` and `http_client_async(Arc<dyn
AsyncHttpClient>)` (emitted by `request_config_setters!` and on `Download`). The
client-specific setters are thin convenience wrappers:
`reqwest_client(c)` => `http_client(Arc::new(ReqwestClient::from(c)))`, and
likewise `ureq_agent` / `reqwest_async_client`, each feature-gated. The old
`ClientOverride` carrier is removed. `set_http_client` (`lib.rs`) forwards an
`Update`'s injected clients to its download.

Because the seam is a trait object, **any** `Arc<dyn HttpClient>` can be injected
— including a user wrapper or a test double — not just the two built-in clients.
When set, `send`/`download_to` dispatch through it instead of building a per-call
client; the `Arc` is reused across requests (sharing its connection pool). The
sync and async injections are independent: injecting one and calling the other
half falls back to that half's per-call client.

What still applies vs defers to the injected client:

- `ReqwestClient`/`ReqwestAsyncClient` built from an injected client
  (`From<reqwest::blocking::Client>` etc.): the per-request `timeout` and
  `headers` are layered onto the request; TLS feature and proxy-env defer to the
  injected client.
- `UreqClient` built from an injected agent: the agent owns its own
  timeout/TLS/proxy, so the per-request `timeout` is applied only to the per-call
  agent and not to an injected agent; extra `request_header`s are still applied
  per request.
- `retries` is independent of the client: it wraps `client.get(...)` in `send`,
  so an injected client is still retried.

### Reuse across paginated requests

The listing walks pages through the sans-io `run_paginated` driver (`backends/mod.rs`), which
calls `send` once per `PageRequest`. Each call passes `&config.client`, so an injected client
(Arc-backed) is reused across all pages, sharing its connection pool; a per-call client is rebuilt
per page. Pagination is bounded by `MAX_RELEASE_PAGES`. `run_paginated_async` is the async sibling,
reusing `send_async`.

### Error mapping (Transport vs status)

A transport-layer failure (connect/timeout/TLS) surfaces through the `?` on the
client's `send()` / `call()`, converted by `From<reqwest::Error>` /
`From<ureq::Error>` into `Error::Transport`. A response with a non-success status is
mapped to a structured status variant by `errors::status_to_error` from the explicit
status check in each `get` (`http_client/reqwest.rs`, `http_client/ureq.rs`): 404 =>
`Error::NotFound { url }`, 401/403 => `Error::Unauthorized { status, url }`, any other
non-2xx => `Error::HttpStatus { status, url }`. Both clients produce the same variants:
for the default ureq agent this needs `http_status_as_error(false)` so the status check
runs, and for an injected ureq agent the `ureq::Error::StatusCode(code)` arm maps the
code instead of letting it fall through to `Transport`. So "could not reach / talk to
the server" is `Transport` and "reached the server, got a bad status" is one of the
status variants.

## Public surface

- `self_update::http` (re-export of the `http` crate); `http_client::header`,
  `http_client::HeaderMap`.
- `self_update::reqwest` / `self_update::ureq` (re-export of each compiled client
  crate; both may be present).
- Builder/`Download` setters: `timeout`, `request_header` / `header`, `retries`,
  `http_client` / `http_client_async`, and the convenience `reqwest_client`,
  `reqwest_async_client`, `ureq_agent`.
- `HttpClient` / `HttpResponse` traits and their async siblings
  `AsyncHttpClient` / `AsyncHttpResponse`; the concrete `ReqwestClient`,
  `ReqwestAsyncClient`, `UreqClient` impls.

## Invariants and regression checklist

- At least one HTTP client must be compiled (no-client is a `compile_error!`);
  both clients can coexist. `async` requires reqwest (auto-satisfied by the
  feature implication).
- The seam traits are object-safe (`Box<dyn HttpClient>` / `Box<dyn HttpResponse>`);
  `json_value` replaces the old generic `json::<T>()`.
- TLS is feature-selected; when both TLS features are on, rustls wins, so
  `cargo build --all-features` builds.
- TLS certificate verification must be enabled on all `HttpClient` implementations
  (documented security contract on the trait).
- The ureq per-call agent explicitly sets `.redirect_auth_headers(RedirectAuthHeaders::Never)`
  rather than relying on the ureq default. This prevents a future ureq version bump from
  silently changing the cross-host redirect auth policy. reqwest strips cross-host auth headers
  by default.
- Custom `HttpClient` implementations must not forward `Authorization` to a different host on a
  redirect (documented security contract on the `HttpClient` and `AsyncHttpClient` traits).
- `retries == 0` means exactly one attempt; the exhaustion boundary is
  `attempts >= retries` (one retry => two attempts).
- Backoff sequence is 100/200/400/800/1600/3200 ms, capped at 3200 from attempt
  5 onward (`100 << attempt.min(5)`); the rising index is fed in-loop.
- The binary download is not retried (`Download` has no `retries`; it calls
  `http_client::get` directly, not `send`).
- An injected client still honors `request_header` and `retries`; for a reqwest
  client it also honors the per-request `timeout`, for a ureq agent the timeout
  defers to the agent. Proxy-env and TLS defer to the injected client.
- Non-success status => a structured status variant (`NotFound` / `Unauthorized` /
  `HttpStatus`), identically on both clients; transport failure => `Error::Transport`.
- Injected clients are `Arc<dyn HttpClient>` and reused across paginated pages.
- s3 feeds quick-xml from the streaming `body_buffered()` reader, not a fully
  buffered `text()` String.

## Tests

- `backends/mod.rs` retry/backoff unit tests: zero-budget attempts once,
  single-retry boundary, exponential-and-capped sequence, in-loop climb to cap,
  later-attempt success, async sibling (`backends/mod.rs:329-530`).
- `backends/github.rs`: `retries_recover_from_transient_failures`,
  `retries_are_exhausted_and_then_error`, `retries=1` boundary, timeout honored,
  pagination follows `Link` (`github.rs:1260-1364`, `764-833`).
- `backends/common.rs`: `insert_header` records invalid name/value, first-error
  wins, valid-then-invalid keeps the valid header.
- `http_client/{reqwest,ureq}.rs`: per-client status-mapping tests, exercised
  through the trait `get`/`json_value`/`text` methods.
- `http_client/ureq.rs`: `per_call_agent_does_not_forward_auth_headers_on_redirect` constructs
  the per-call agent config with `RedirectAuthHeaders::Never` and asserts `config.redirect_auth_headers() == RedirectAuthHeaders::Never`.
  Fails if the explicit `.redirect_auth_headers(...)` call is removed.
- `backends/github.rs`: `injected_fake_http_client_drives_a_backend_through_the_trait`
  (a `FakeClient` test double injected via `.http_client(Arc::new(...))` records
  the URL and returns a canned `Box<dyn HttpResponse>`), and
  `http_traits_are_object_safe` (a `Box<dyn HttpClient>` / `Box<dyn HttpResponse>`
  compile assertion).
- `backends/s3.rs`: `parse_s3_response_parses_from_streaming_body_buffered` drives
  the XML parser from a trait `body_buffered()` reader.
- `errors.rs`: boxed `source()` mirroring for `Http` and siblings.

## Related

- `transport-control.md`
- `error-network-vs-http-semantics.md`
- `ref-feature-flags.md`

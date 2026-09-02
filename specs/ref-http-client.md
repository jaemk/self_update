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
`headers() -> &HeaderMap<HeaderValue>`, `body(self: Box<Self>) -> Box<dyn Read>`,
and a `body_buffered` default wrapping `body()` in a `BufReader`. The former
`json_value` / `text` methods are removed: the crate parses JSON/XML from the
body reader itself, so a custom transport implements only `headers()` +
`body()`. It is implemented for `reqwest::blocking::Response` and ureq's
`Response<Body>`.

The async path (reqwest + tokio only) has the sibling object-safe traits
`AsyncHttpClient`/`AsyncHttpResponse` (`http_client/mod.rs`). `AsyncHttpResponse`
exposes `headers()`, `text()`, and `bytes_stream() -> BoxStream<Result<Bytes>>`;
`download_to_async` drives `bytes_stream()` rather than leaking a concrete
`reqwest::Response`. `default_async_client()` is always reqwest. The `bytes`
crate is a direct optional dep gated under `async`.

Headers use the `http` crate types throughout. `src/http_client/mod.rs:HttpClient`
re-exports `http::header` and `http::HeaderMap`; the whole `http` crate is
re-exported as `self_update::http` (`src/lib.rs:http_client`) so consumers can name header
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
(`src/macros.rs:request_config_setters`), writing into a `RequestConfig` (`src/backends/common.rs:RequestConfig`)
that holds `timeout`, `headers`, `retries`, an injected `client`
(`Option<Arc<dyn HttpClient>>` / `Option<Arc<dyn AsyncHttpClient>>`), and a deferred `header_error`.

- `timeout` sets a per-request timeout, default none, applied to every request
  the builder makes including the download (`src/macros.rs:timeout`).
- `request_header(name, value)` inserts one extra header; a repeated name
  overwrites. It is infallible at call time: an invalid name/value is stored as
  the first `header_error` (`src/backends/common.rs:header_error`) and surfaced from
  `build()` as `Error::InvalidHeader` via `check()` (`src/backends/common.rs:check`).
- `retries` is the number of retries (default 0 = one attempt); the download's
  request-establishment phase is retried under the same budget, but mid-stream
  transfer errors are not retried. It is a no-op on the custom backend
  (`src/macros.rs:retries`).

The retry loop lives in `backends/mod.rs`, not in the http_client module.
`send` (`src/backends/mod.rs:send`) merges `config.headers` over the backend's
base headers, then calls `retry` with `client.get(...)` as the attempt and a
closure that logs a warning and sleeps `backoff` ms between tries. `retry`
(`src/backends/mod.rs:retry`) runs the attempt, and on error returns immediately
once `attempts >= retries` **or the error is `Error::RateLimited`** (checked via
`is_rate_limited`, `src/backends/mod.rs:is_rate_limited`), otherwise sleeps
`retry_backoff_ms(attempts)` and increments. So `retries == 0` attempts exactly
once; the budget boundary is `>=`. Any failed attempt still consumes budget --
**except** `Error::RateLimited`, which always returns on the very first attempt
regardless of `retries`, since the request's quota is already exhausted and a
sub-second backoff cannot make more of it appear (see `ref-errors.md` /
`auth-token-from-env.md` AUTH-2-9). A permanent 404 (or any other non-`RateLimited`
error) still consumes the full budget as before.

Backoff is `retry_backoff_ms(attempt) = 100u64 << attempt.min(5)`
(`src/backends/mod.rs:retry_backoff_ms`): 100, 200, 400, 800, 1600, 3200 ms, capped at
3200 ms for attempt 5 and beyond. The in-loop attempt index feeds the rising
backoff (not just index 0). `send_async` (`src/backends/mod.rs:send_async`) / `retry_async`
(`src/backends/mod.rs:retry_async`) are the async siblings, using `tokio::time::sleep`,
short-circuiting on `Error::RateLimited` the same way as the sync `retry`;
the log runs synchronously between tries so the error is never held across the
await.

### Root certificates and auth-token scope

`self_update::Certificate` is an opaque root-CA certificate (PEM or DER).
`add_root_certificate(Certificate)` on the `Update`/`ReleaseList` builders and on
`Download` adds it to the per-call client's trust store, so a private/internal CA
can be trusted without injecting a whole pre-built client. A malformed
certificate (or a client that cannot be built with it) surfaces as
`Error::InvalidCertificate { source }` from `build()` or `download_to` /
`download_to_async`. Each client slot is materialized independently
(`RequestConfig::build_client`, `backends/common.rs`): the certs build a client
only for a slot with no injected client, so an injected client keeps its own TLS
trust and the other slot still trusts the added certs.

The `auth_token` is sent only to requests whose host matches the backend's
configured API host (or an `allow_auth_host(host)` entry), over https.
`dangerously_allow_non_https_auth_forwarding()` drops the https requirement for a
host-matched request.

### Proxy

Both clients honor `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY`. reqwest does this
automatically. ureq's per-call agent sets `.proxy(ureq::Proxy::try_from_env())`
explicitly (`src/http_client/ureq.rs:build_call_agent`). Proxy-from-env applies only to the
per-call client; an injected client is left to its own proxy config (see below).

`proxy(url)` on the `Update`/`ReleaseList` builders and on `Download` configures a proxy
programmatically, for the authenticated case the env vars cannot cover: the URL may embed
credentials (`http://user:pass@host:port`), which both clients parse and send as
`Proxy-Authorization`. Only HTTP CONNECT proxies are supported (SOCKS is a non-goal). Like
the root certificates, it is materialized into the client at `build()` time by
`RequestConfig::build_client` (`src/backends/common.rs:RequestConfig`) -- both knobs feed one
`ClientConfig` and one client builder (`src/http_client/mod.rs:build_configured_client`), and a
slot with an injected client is skipped. An unparseable URL is deferred to
`Error::InvalidProxy { source }` from `build()` / `download_to` /
`download_to_async`, with the password redacted (`src/errors.rs:redact_proxy_url`,
`src/errors.rs:proxy_error_message`) -- also in every `Debug` rendering of the config.

Lane differences: on reqwest the programmatic proxy is `Proxy::all` *added to* the
env-var proxies (first match wins), so both are active; on a ureq-only build the agent has a
single proxy slot, so the configured proxy replaces the env-var one
(`src/http_client/ureq.rs:build_call_agent`). `build_download`
(`src/update.rs:build_download`) forwards the proxy to the download, so the listing and the
asset fetch take the same route.

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
mapped to a structured status variant by `errors::status_to_error_with_headers` (which
reads the response's rate-limit headers and delegates to the pure `classify_status`; see
`ref-errors.md`) from the explicit status check in each `get`
(`http_client/reqwest.rs`, `http_client/ureq.rs`): 404 => `Error::NotFound { url }`;
429 (always), or 403 with a spent quota or a usable `Retry-After` => `Error::RateLimited
{ status, url, reset_at, retry_after }`; 401, or 403 with neither rate-limit signal =>
`Error::Unauthorized { status, url }`; any other non-2xx => `Error::HttpStatus { status,
url }`.

All three client lanes produce the same variants for a given status + headers:
- The **default (per-call) ureq agent** is built with `.http_status_as_error(false)`
  (`build_call_agent`, `src/http_client/ureq.rs:build_call_agent`) so the explicit status check at the bottom of `get`
  runs with the response headers in hand.
- An **injected ureq agent** keeps ureq's own default `http_status_as_error(true)` at the
  agent level, but `get` applies a **per-request** override on the request builder --
  `req.config().http_status_as_error(false).build()` (`src/http_client/ureq.rs:UreqClient::get`) -- so it also
  reaches the same header-aware check instead of failing early with a headerless
  `ureq::Error::StatusCode`. This closed a prior gap where an injected agent's spent-quota
  403 fell back to `Unauthorized` (no headers reachable from `StatusCode`). The
  `Err(ureq::Error::StatusCode(code)) if is_injected` arm (`src/http_client/ureq.rs:UreqClient::get`) is retained
  only as a defensive fallback for a future ureq that stops honoring the per-request
  override; it maps through the header-less `status_to_error` and so still cannot produce
  `RateLimited`.
- **reqwest** always has the response (and its headers) in hand at the status check.

So "could not reach / talk to the server" is `Transport` and "reached the server, got a
bad status" is one of the status variants (`NotFound` / `Unauthorized` / `RateLimited` /
`HttpStatus`), identically across all three lanes.

## Public surface

- `self_update::http` (re-export of the `http` crate); `http_client::header`,
  `http_client::HeaderMap`.
- `self_update::reqwest` / `self_update::ureq` (re-export of each compiled client
  crate; both may be present).
- Builder/`Download` setters: `timeout`, `request_header`, `retries`,
  `http_client` / `http_client_async`, `add_root_certificate`, `proxy`, and the
  convenience `reqwest_client`, `reqwest_async_client`, `ureq_agent`; plus
  `allow_auth_host` and `dangerously_allow_non_https_auth_forwarding` on the
  builders.
- `self_update::Certificate` (opaque PEM/DER root CA).
- `HttpClient` / `HttpResponse` traits and their async siblings
  `AsyncHttpClient` / `AsyncHttpResponse`; the concrete `ReqwestClient`,
  `ReqwestAsyncClient`, `UreqClient` impls.

## Invariants and regression checklist

- At least one HTTP client must be compiled (no-client is a `compile_error!`);
  both clients can coexist. `async` requires reqwest (auto-satisfied by the
  feature implication).
- The seam traits are **object-safe** (`Box<dyn HttpClient>` / `Box<dyn
  HttpResponse>`); the sync `HttpResponse` surface is `headers()` + `body()`
  (plus the defaulted `body_buffered()`), with no `json_value` / `text`.
- TLS is feature-selected; when both TLS features are on, rustls wins, so
  `cargo build --all-features` builds.
- `retries == 0` means exactly one attempt; the exhaustion boundary is
  `attempts >= retries` (one retry => two attempts). **Exception:** `Error::RateLimited`
  always returns after the first attempt regardless of `retries` (`is_rate_limited`,
  `src/backends/mod.rs:is_rate_limited`, checked by both `retry` and `retry_async`) -- the request's
  quota is already spent, so retrying cannot succeed and would only spend more of it.
- Backoff sequence is 100/200/400/800/1600/3200 ms, capped at 3200 from attempt
  5 onward (`100 << attempt.min(5)`); the rising index is fed in-loop. Never reached for
  `Error::RateLimited`, which short-circuits before any backoff is scheduled.
- The binary download's request-establishment phase is retried under the `retries`
  budget (via `send`), with the same `RateLimited` short-circuit; mid-stream transfer
  errors are not retried.
- An injected client still honors `request_header` and `retries`; for a reqwest
  client it also honors the per-request `timeout`, for a ureq agent the timeout
  defers to the agent. Proxy-env, a configured `proxy(url)`, `add_root_certificate` and TLS
  all defer to the injected client (per slot: injecting the sync client does not stop the
  async slot from being built from the configuration, and vice versa).
- A configured `proxy(url)` and `add_root_certificate` certs are applied to the **same**
  crate-built client, and a proxy alone is enough to trigger that build; the proxy is
  forwarded to the download, and its password never appears in a `Debug` or an error.
- Non-success status => a structured status variant (`NotFound` / `Unauthorized` /
  `RateLimited` / `HttpStatus`), identically on **all three** client lanes -- default ureq
  agent, injected ureq agent (via a per-request `http_status_as_error(false)` override,
  `src/http_client/ureq.rs:UreqClient::get`), and reqwest; transport failure => `Error::Transport`. See
  `ref-errors.md` for the full classification rule (429 always `RateLimited`; 403 on a spent
  quota or a usable `Retry-After`).
- Injected clients are `Arc<dyn HttpClient>` and reused across paginated pages.
- s3 feeds quick-xml from the streaming `body_buffered()` reader, not a fully
  buffered `text()` String.

## Tests

- `backends/mod.rs` retry/backoff unit tests (`mod tests`): zero-budget attempts once,
  single-retry boundary, exponential-and-capped sequence, in-loop climb to cap,
  later-attempt success, async sibling. The
  `RateLimited` short-circuit: `retry_does_not_retry_a_rate_limited_error`,
  `retry_still_consumes_the_budget_for_a_non_rate_limited_error`,
  `retry_async_does_not_retry_a_rate_limited_error`,
  `retry_async_still_consumes_the_budget_for_a_non_rate_limited_error`.
- `backends/github.rs`: `retries_recover_from_transient_failures`,
  `retries_are_exhausted_and_then_error`, `retries=1` boundary, timeout honored,
  pagination follows `Link` (`src/backends/github.rs:fetch_all_releases_follows_link_pagination`,
  `src/backends/github.rs:fetch_all_releases_async_follows_pagination`).
- `backends/common.rs`: `insert_header` records invalid name/value, first-error
  wins, valid-then-invalid keeps the valid header.
- `http_client/{reqwest,ureq}.rs`: per-client status-mapping tests, exercised
  through the trait `get` method; `build_with_certs_rejects_non_pem` for the
  certificate path. `http_client/ureq.rs` additionally pins the injected-agent
  classification gap closure: `injected_agent_sees_rate_limit_headers`,
  `injected_agent_with_status_error_disabled_sees_rate_limit_headers`,
  `injected_agent_no_status_error_falls_through_to_is_success_check`,
  `default_agent_path_maps_statuses_identically_to_injected`.
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

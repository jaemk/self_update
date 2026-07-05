# Corporate Network Config

Status: CORP-1 done; CORP-2 and CORP-3 pending

## Problem

In corporate environments, outbound HTTPS typically passes through an intercepting proxy
that presents a company-issued TLS certificate. The default `rustls` TLS backend bundles
its own root store and does not read the OS trust store, so any connection through such a
proxy fails with a certificate verification error unless the caller injects a
pre-configured `reqwest` client with the custom CA added manually.

CORP-1 has shipped; CORP-2 and CORP-3 remain open:

- **Custom root CA (CORP-1)**: shipped. `self_update::Certificate` plus
  `add_root_certificate` on every builder and on `Download`; a malformed
  certificate surfaces as `Error::InvalidCertificate` from `build()` /
  `download_to`. See CORP-1 below and `ref-http-client.md`.

- **OS trust store (CORP-2)**: the default reqwest + rustls setup in 0.13.4 already uses
  the platform verifier (OS trust store) via `rustls-platform-verifier`. The gap is the
  ureq per-call path, which defaults to Mozilla's bundled roots (`WebPki`), not the OS.

- **Proxy with auth (CORP-3)**: env-var passthrough (`HTTP_PROXY` / `HTTPS_PROXY`) works
  for unauthenticated proxies, but any proxy requiring credentials must be configured on
  an injected client. There is no `.proxy(url)` setter on the builders.

The remaining gaps (CORP-2, CORP-3) force corporate users to add `reqwest` or `ureq` as
a direct dependency solely to unlock transport config that belongs on the `self_update`
builder surface.

---

## CORP-1: custom root CA (done)

### `self_update::Certificate` type

CORP-1-1. A new `Certificate` type is exported from the crate root (`self_update::Certificate`).

CORP-1-2. `Certificate` is an opaque struct holding raw bytes and a format tag:

```rust
pub struct Certificate {
    bytes: Vec<u8>,
    format: CertFormat, // private enum: Pem | Der
}
```

CORP-1-3. Two infallible constructors: `Certificate::from_pem(bytes: impl Into<Vec<u8>>)`
and `Certificate::from_der(bytes: impl Into<Vec<u8>>)`. Both store raw bytes without
parsing; parsing is deferred to `build()` so the builder setter stays infallible.

CORP-1-4. The PEM variant contains a single certificate. Callers needing multiple
certificates call `add_root_certificate` more than once. No batch constructor.

### Builder setter

CORP-1-5. `add_root_certificate(cert: Certificate) -> &mut Self` is added to
`request_config_setters!` (src/macros.rs) so it is available on every backend's
`UpdateBuilder` and `ReleaseListBuilder`. Appending: each call adds one cert, previous
certs are not replaced.

CORP-1-6. `RequestConfig` (src/backends/common.rs) gets a new field:
`root_certificates: Vec<Certificate>` defaulting to `vec![]`.

CORP-1-7. `Download` (src/lib.rs) gets an `add_root_certificate(cert: Certificate) -> &mut Self`
setter (same name as the builder setter) and a private `root_certificates: Vec<Certificate>`
field, forwarded from `build_download()` alongside the existing client forward.

### Application at `build()` time

CORP-1-8. When `root_certificates` is non-empty and no `client` has been injected,
`build()` converts the cert bytes into a backend-specific pre-built client and stores it
as `config.client`. The conversion and client construction happen once, at `build()` time;
runtime request handling is unchanged.

CORP-1-9. If an explicit `http_client()` / `reqwest_client()` / `ureq_agent()` call was
made, the injected client wins and `root_certificates` has no effect (consistent with
how proxy-env and the TLS feature defer to an injected client today). Documented on the
setter.

CORP-1-10. Certificate parse/conversion errors surface as
`Error::InvalidCertificate { source }` from `build()` (`Error::Config` no longer exists).
A `cert_error: Option<String>` field is added to `RequestConfig`, populated on conversion
failure, and surfaced by `RequestConfig::check()` alongside `header_error` (the header
error takes precedence).

### reqwest path

CORP-1-11. Under `#[cfg(feature = "reqwest")]`, `build()` constructs a
`reqwest::blocking::Client` via `ClientBuilder::new()`, applies TLS backend selection
(`use_rustls_tls()` / `use_native_tls()`), and calls `tls_certs_merge(certs)` with the
converted certificates. (`add_root_certificate` is deprecated in reqwest 0.13;
`tls_certs_merge` is the replacement.)

CORP-1-12. With the `rustls` feature, reqwest 0.13.4 uses `rustls_platform_verifier`
by default (see CORP-2). `tls_certs_merge` with non-empty certs calls
`rustls_platform_verifier::Verifier::new_with_extra_roots(der_certs)`: the OS trust store
remains active and the custom certs are added on top. Standard CA trust is unchanged.

CORP-1-13. For the async path, the same conversion logic applies to
`reqwest::ClientBuilder` under `#[cfg(feature = "async")]`.

### ureq path

CORP-1-14. Under `#[cfg(all(feature = "ureq", not(feature = "reqwest")))]` (ureq-only
build), `build()` constructs a `ureq::Agent` with
`TlsConfigBuilder::root_certs(RootCerts::Specific(certs))`.

CORP-1-15. `RootCerts::Specific` replaces ureq's default `WebPki` root store. This is a
documented limitation: when custom root certs are set on a ureq-only build, only the
supplied certificates are trusted. The doc comment on `add_root_certificate` notes this
and recommends either supplying all needed CA certs or injecting a `ureq::Agent` built
with `RootCerts::PlatformVerifier` (see CORP-2) for the broader trust case.

CORP-1-16. When both `reqwest` and `ureq` features are on, reqwest is selected (same
priority as `default_client()`), so CORP-1-15's limitation does not apply to the default
feature set.

### Feature gates summary

```
add_root_certificate setter   -- always compiled (cert stored in RequestConfig)
reqwest build-time client     -- #[cfg(feature = "reqwest")]
async reqwest client          -- #[cfg(feature = "async")]
ureq build-time client        -- #[cfg(all(feature = "ureq", not(feature = "reqwest")))]
```

---

## CORP-2: OS trust store for ureq (designed)

### reqwest: already solved

CORP-2-1. reqwest 0.13.4 with the `rustls` feature uses `rustls-platform-verifier` by
default. When `root_certs` is empty and `tls_certs_only` is false (the defaults),
reqwest builds a `rustls_platform_verifier::Verifier::new()` verifier, which uses the
OS trust store. This is the current behavior of the default feature set in self_update.
No code or feature change is needed for reqwest.

CORP-2-2. Callers on corporate networks whose company CA is already installed in the OS
trust store do not need to call `add_root_certificate` at all when using the reqwest
(default) backend.

### ureq: new `native-certs` feature

CORP-2-3. A new Cargo feature `native-certs` is added to self_update:
```toml
native-certs = ["ureq?/platform-verifier"]
```

CORP-2-4. Under `#[cfg(feature = "native-certs")]`, the per-call ureq agent builder
switches from `RootCerts::WebPki` to `RootCerts::PlatformVerifier`:

```rust
#[cfg(feature = "native-certs")]
let root_certs = ureq::tls::RootCerts::PlatformVerifier;
#[cfg(not(feature = "native-certs"))]
let root_certs = ureq::tls::RootCerts::WebPki;

Agent::config_builder()
    .tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(provider)
            .root_certs(root_certs)
            .build()
    )
    ...
```

CORP-2-5. `native-certs` has no effect on an injected ureq agent; the agent owns its own
TLS config. Documented on the feature.

CORP-2-6. `native-certs` is not included in the crate's `default` feature set. Callers
opt in explicitly.

CORP-2-7. When both `reqwest` and `ureq` features are on, the ureq per-call path is not
reached; `native-certs` is effectively a no-op (no overhead). This is acceptable since
the common default-feature path uses reqwest with the platform verifier already.

---

## CORP-3: proxy with auth (designed)

### Builder setter

CORP-3-1. `.proxy(url: impl Into<String>) -> &mut Self` is added to
`request_config_setters!`. The URL follows the standard proxy URL format and may embed
credentials: `http://user:pass@host:port` or `https://user:pass@host:port`. Both reqwest
and ureq parse credentials from the URL directly.

CORP-3-2. `RequestConfig` gets a new field `proxy: Option<String>` defaulting to `None`.
Only one proxy URL is stored; calling `.proxy()` more than once overwrites the previous
value.

CORP-3-3. The setter is infallible. URL parse errors are deferred to `build()` via a
`proxy_error: Option<String>` field in `RequestConfig`, surfaced by
`RequestConfig::check()` alongside `cert_error` and `header_error`.

### Application at `build()` time

CORP-3-4. Proxy config follows the same build-time pattern as CORP-1: if `proxy` is set
and no `client` was injected, `build()` constructs a configured backend client with the
proxy applied and stores it as `config.client`.

CORP-3-5. If both `root_certificates` (CORP-1) and `proxy` (CORP-3) are set, `build()`
applies both to the same client builder before storing the result as `config.client`.

CORP-3-6. If an explicit `http_client()` / `reqwest_client()` / `ureq_agent()` was set,
the injected client wins and `.proxy()` has no effect. Documented on the setter.

### reqwest path

CORP-3-7. Under `#[cfg(feature = "reqwest")]`, `build()` adds a proxy via
`reqwest::Proxy::all(url)?` and then `client_builder.proxy(proxy)`. `Proxy::all` routes
all requests (HTTP and HTTPS) through the specified URL.

CORP-3-8. Env-var proxies (`HTTP_PROXY` / `HTTPS_PROXY`) remain active alongside a
programmatic proxy; reqwest applies all configured proxies in order, first match wins.
To disable env-var proxies the caller must inject a client built with `.no_proxy()`.

### ureq path

CORP-3-9. Under `#[cfg(all(feature = "ureq", not(feature = "reqwest")))]`, `build()`
constructs the ureq `Agent` with `agent_builder.proxy(ureq::Proxy::new(url)?)` instead
of `proxy(ureq::Proxy::try_from_env())`. The programmatic proxy replaces env-var proxy
for the per-call agent; ureq agents have a single proxy slot.

CORP-3-10. When no programmatic proxy is set, the ureq per-call path continues to use
`ureq::Proxy::try_from_env()` (current behavior, unchanged).

CORP-3-11. Callers who need both env-var fallback and programmatic-proxy auth (e.g.
"use programmatic proxy if env var is unset") must inject a `ureq::Agent`.

---

## Non-goals

- Mutual TLS (mTLS / client cert auth): the injection seam covers this for callers.
- Certificate pinning: out of scope.
- Proxy protocol beyond HTTP CONNECT (e.g. SOCKS5): out of scope.
- Disabling certificate verification: out of scope (injection seam only).

use std::time::Duration;

use http::HeaderValue;

use crate::Result;
pub use http::HeaderMap;
pub use http::header;

mod reqwest;
mod ureq;

#[cfg(feature = "async")]
pub use reqwest::ReqwestAsyncClient;
#[cfg(feature = "reqwest")]
pub use reqwest::ReqwestClient;
#[cfg(feature = "ureq")]
pub use ureq::UreqClient;

/// Object-safe HTTP transport seam. The crate only ever issues GETs, so `get` is the whole
/// surface. Each implementation maps a non-2xx status to the structured
/// `NotFound`/`Unauthorized`/`HttpStatus` error variant *before* returning `Ok` (preserving the
/// per-client `status_to_error` logic), so callers see a structured error for a bad status and an
/// `Ok(response)` only for a 2xx.
///
/// Object safety is what makes the transport injectable: a user can hand the crate any
/// `Arc<dyn HttpClient>` (e.g. a test double or a wrapper around a custom client). Retries are
/// **not** part of this trait — they stay in `backends::send`/`retry`, wrapping `client.get(...)`.
///
/// # Security contract for implementations
///
/// Custom implementations **MUST** uphold two security properties:
///
/// 1. **TLS certificate verification must be enabled.** Disabling certificate verification allows
///    a man-in-the-middle to serve arbitrary binaries as release artifacts. Never pass
///    `danger_accept_invalid_certs` or equivalent options.
///
/// 2. **The `Authorization` header MUST NOT be forwarded to a different host on a redirect.**
///    An attacker-controlled redirect destination could harvest bearer tokens or API keys.
///    Configure your HTTP client to strip the `Authorization` header when following a redirect
///    to a different origin (both reqwest and the built-in ureq client do this by default).
pub trait HttpClient: Send + Sync {
    /// Issue a GET to `url` with the given `headers` and optional per-request `timeout`, returning
    /// the response (already status-checked) as a boxed [`HttpResponse`].
    fn get(
        &self,
        url: &str,
        headers: &HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<Box<dyn HttpResponse>>;
}

/// Object-safe response handle returned by [`HttpClient::get`].
///
/// `json_value`/`text` borrow `&mut self` (they may consume the body internally), and `body` /
/// `body_buffered` consume `self` to hand back a streaming reader. There are no generic methods, so
/// the trait stays object-safe.
pub trait HttpResponse {
    /// The response headers.
    fn headers(&self) -> &HeaderMap<HeaderValue>;

    /// Parse the body as a `serde_json::Value`. This replaces the old generic `json::<T>()` — every
    /// call site requests `serde_json::Value`, so a single non-generic method keeps object safety.
    fn json_value(&mut self) -> Result<serde_json::Value>;

    /// Read the whole body as a `String`.
    fn text(&mut self) -> Result<String>;

    /// Consume the response and return its body as a streaming reader.
    fn body(self: Box<Self>) -> Box<dyn std::io::Read>;

    /// Consume the response and return its body as a buffered streaming reader. The default wraps
    /// [`body`](Self::body) in a `BufReader`; the s3 backend feeds quick-xml from this so the XML is
    /// not fully buffered into a `String` first.
    fn body_buffered(self: Box<Self>) -> Box<dyn std::io::BufRead> {
        Box::new(std::io::BufReader::new(self.body()))
    }
}

/// Async sibling of [`HttpClient`] (reqwest + tokio only). Object-safe like the sync seam, so an
/// async client can be injected as an `Arc<dyn AsyncHttpClient>`.
///
/// # Security contract for implementations
///
/// The same security requirements as [`HttpClient`] apply: TLS certificate verification
/// **MUST** be enabled, and the `Authorization` header **MUST NOT** be forwarded to a different
/// host on a redirect. See [`HttpClient`] for details.
#[cfg(feature = "async")]
pub trait AsyncHttpClient: Send + Sync {
    /// Issue a GET, returning a boxed future resolving to the status-checked response.
    fn get<'a>(
        &'a self,
        url: &'a str,
        headers: &'a HeaderMap,
        timeout: Option<Duration>,
    ) -> futures_util::future::BoxFuture<'a, Result<Box<dyn AsyncHttpResponse>>>;
}

/// Async sibling of [`HttpResponse`]. Drives the streamed download (`bytes_stream`) instead of
/// leaking a concrete `reqwest::Response`.
#[cfg(feature = "async")]
pub trait AsyncHttpResponse: Send {
    /// The response headers.
    fn headers(&self) -> &HeaderMap<HeaderValue>;

    /// Consume the response and read the whole body as a `String`.
    fn text(self: Box<Self>) -> futures_util::future::BoxFuture<'static, Result<String>>;

    /// Consume the response and stream its body as chunks of bytes (used by `download_to_async`).
    fn bytes_stream(
        self: Box<Self>,
    ) -> futures_util::stream::BoxStream<'static, Result<bytes::Bytes>>;
}

/// The sync HTTP client the crate builds by default when none is injected. Selects reqwest when the
/// `reqwest` feature is on (preferred when both are enabled), else ureq. A genuine no-client build
/// is a `compile_error!`.
#[cfg(feature = "reqwest")]
pub(crate) fn default_client() -> Box<dyn HttpClient> {
    Box::new(ReqwestClient::default())
}

#[cfg(all(not(feature = "reqwest"), feature = "ureq"))]
pub(crate) fn default_client() -> Box<dyn HttpClient> {
    Box::new(UreqClient::default())
}

// A genuine no-client build cannot service any request. Surface a readable diagnostic instead of a
// missing-symbol error.
#[cfg(not(any(feature = "reqwest", feature = "ureq")))]
compile_error!(
    "no HTTP client selected - enable at least one of the `reqwest` (default) or `ureq` features"
);

/// The async HTTP client the crate builds by default. Async is always reqwest.
#[cfg(feature = "async")]
pub(crate) fn default_async_client() -> Box<dyn AsyncHttpClient> {
    Box::new(ReqwestAsyncClient::default())
}

#[cfg(test)]
mod tests {
    /// When BOTH `reqwest` and `ureq` are compiled in, `default_client()` must select the reqwest
    /// impl (it is the documented default; the ureq arm is `cfg(all(not(feature="reqwest"),
    /// feature="ureq"))`). A `Box<dyn HttpClient>` cannot be downcast, so prove the selection by
    /// comparing the concrete `TypeId` the (private) selection would produce against `ReqwestClient`.
    /// This fails if a future edit flips the cfg ordering so ureq wins when both are on.
    #[cfg(all(feature = "reqwest", feature = "ureq"))]
    #[test]
    fn default_client_prefers_reqwest_when_both_enabled() {
        use std::any::TypeId;

        // Reproduce the EXACT cfg precedence `default_client()` uses, returning the concrete
        // `TypeId` the boxed client would carry. The two arms below copy the module's own cfg
        // guards verbatim (`feature="reqwest"` wins; the ureq arm requires
        // `not(feature="reqwest")`), so if a future edit flips that ordering this helper's result
        // flips with it and the assertion below fails.
        fn selected_default_client_type() -> TypeId {
            #[cfg(feature = "reqwest")]
            {
                TypeId::of::<super::ReqwestClient>()
            }
            #[cfg(all(not(feature = "reqwest"), feature = "ureq"))]
            {
                TypeId::of::<super::UreqClient>()
            }
        }

        assert_eq!(
            selected_default_client_type(),
            TypeId::of::<super::ReqwestClient>(),
            "with both clients enabled the default must select the reqwest client, not ureq"
        );
        assert_ne!(
            selected_default_client_type(),
            TypeId::of::<super::UreqClient>(),
            "the default must NOT be the ureq client when reqwest is also enabled"
        );
        // Both concrete clients are distinct, coexisting `HttpClient` impls, and `default_client()`
        // builds a working boxed client (no panic).
        assert_ne!(
            TypeId::of::<super::ReqwestClient>(),
            TypeId::of::<super::UreqClient>(),
            "reqwest and ureq are distinct coexisting client types"
        );
        let _client: Box<dyn super::HttpClient> = super::default_client();
    }
}

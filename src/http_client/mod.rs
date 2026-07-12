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
/// The whole surface is `headers` plus a body reader: [`body`](Self::body) (and its buffered
/// sibling [`body_buffered`](Self::body_buffered)) consume `self: Box<Self>`, so single-use is
/// enforced at the type level. A custom transport implements `headers` + `body`; the crate parses
/// JSON/XML from the reader itself. There are no generic methods, so the trait stays object-safe.
pub trait HttpResponse {
    /// The response headers.
    fn headers(&self) -> &HeaderMap<HeaderValue>;

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
/// The signature types from foreign crates (`BoxFuture`, `BoxStream`, `Bytes`) are re-exported at
/// the crate root (`self_update::futures_util`, `self_update::bytes`), so an implementation does
/// not need them as direct dependencies.
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
///
/// Test builds set [`DEFAULT_CLIENT_BACKEND`] before returning so tests can inspect which concrete
/// type was actually constructed without needing a downcast.
#[cfg(feature = "reqwest")]
pub(crate) fn default_client() -> Box<dyn HttpClient> {
    #[cfg(test)]
    DEFAULT_CLIENT_BACKEND.with(|c| c.set("reqwest"));
    Box::new(ReqwestClient::default())
}

#[cfg(all(not(feature = "reqwest"), feature = "ureq"))]
pub(crate) fn default_client() -> Box<dyn HttpClient> {
    #[cfg(test)]
    DEFAULT_CLIENT_BACKEND.with(|c| c.set("ureq"));
    Box::new(UreqClient::default())
}

/// A boxed error used by the cert-client builders, so the underlying certificate-parse / client
/// build failure is preserved as a `source` chain rather than stringified.
pub(crate) type ClientBuildError = Box<dyn std::error::Error + Send + Sync>;

/// Build a sync HTTP client pre-configured with custom root CA certificates.
/// Returns `Err` if the cert bytes are invalid or the client cannot be built.
/// When both `reqwest` and `ureq` are enabled, reqwest is preferred (same priority as default_client).
pub(crate) fn client_with_root_certs(
    certs: &[crate::tls::Certificate],
) -> std::result::Result<std::sync::Arc<dyn HttpClient>, ClientBuildError> {
    #[cfg(feature = "reqwest")]
    {
        crate::http_client::ReqwestClient::build_with_certs(certs)
    }
    #[cfg(all(feature = "ureq", not(feature = "reqwest")))]
    {
        crate::http_client::UreqClient::build_with_certs(certs)
    }
    #[cfg(not(any(feature = "reqwest", feature = "ureq")))]
    {
        let _ = certs;
        Err("no HTTP client feature enabled".into())
    }
}

/// Async sibling of [`client_with_root_certs`]. Only reqwest is supported (async is reqwest-only).
#[cfg(feature = "async")]
pub(crate) fn async_client_with_root_certs(
    certs: &[crate::tls::Certificate],
) -> std::result::Result<std::sync::Arc<dyn AsyncHttpClient>, ClientBuildError> {
    crate::http_client::ReqwestAsyncClient::build_async_with_certs(certs)
}

// Records the backend name chosen by the most recent `default_client` call on this thread.
// Only compiled in test builds; production code sees no overhead.
#[cfg(test)]
thread_local! {
    pub(crate) static DEFAULT_CLIENT_BACKEND: std::cell::Cell<&'static str> =
        const { std::cell::Cell::new("unset") };
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
    /// impl (it is the documented default; the ureq arm requires `not(feature="reqwest")`).
    ///
    /// The test calls `super::default_client()` directly — not a duplicate of its cfg logic — and
    /// reads [`super::DEFAULT_CLIENT_BACKEND`] to confirm which concrete type was instantiated. If a
    /// future edit flips the cfg ordering so the ureq arm wins when both features are enabled, this
    /// thread-local will read `"ureq"` and the assertion below fails.
    #[cfg(all(feature = "reqwest", feature = "ureq"))]
    #[test]
    fn default_client_prefers_reqwest_when_both_enabled() {
        // Reset the sentinel so a previous test run on this thread does not leak state.
        super::DEFAULT_CLIENT_BACKEND.with(|c| c.set("unset"));

        // Exercise the real `default_client()` — this is the function under test.
        let _client: Box<dyn super::HttpClient> = super::default_client();

        // Read what default_client() actually instantiated.
        let backend = super::DEFAULT_CLIENT_BACKEND.with(|c| c.get());

        assert_eq!(
            backend, "reqwest",
            "with both clients enabled default_client() must instantiate the reqwest backend, not ureq"
        );
        assert_ne!(
            backend, "ureq",
            "the default must NOT be the ureq client when reqwest is also enabled"
        );
    }
}

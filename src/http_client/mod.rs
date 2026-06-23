use http::HeaderValue;
use serde::de::DeserializeOwned;

use crate::Result;
pub use http::HeaderMap;
pub use http::header;

mod reqwest;
mod ureq;

#[cfg(feature = "reqwest")]
pub use reqwest::*;
#[cfg(feature = "ureq")]
pub use ureq::*;

/// The concrete async HTTP response type used by the `async` update API. Async support is
/// reqwest-only, so this is just `reqwest::Response` (no trait abstraction is needed for a single
/// client).
#[cfg(feature = "async")]
pub type AsyncResponse = ::reqwest::Response;

/// An optional, user-supplied HTTP client to use instead of the per-call client the crate builds.
///
/// Client-agnostic container with client-specific fields: only the field for the active client
/// (and, under `async`, the async reqwest client) is compiled. All three client types are
/// Arc-backed, so storing/cloning shares the connection pool. When a field is set, the matching
/// `get`/`get_async` reuses that client; per-request headers and (for reqwest) the per-request
/// timeout are still layered on, but proxy-env and the TLS feature are left to the injected client.
#[derive(Clone, Debug, Default)]
pub struct ClientOverride {
    #[cfg(feature = "reqwest")]
    pub(crate) blocking: Option<::reqwest::blocking::Client>,
    #[cfg(feature = "async")]
    pub(crate) r#async: Option<::reqwest::Client>,
    #[cfg(feature = "ureq")]
    pub(crate) agent: Option<::ureq::Agent>,
}

pub trait HttpResponse {
    fn headers(&self) -> &HeaderMap<HeaderValue>;

    fn body(self) -> impl std::io::Read;

    fn json<T: DeserializeOwned>(self) -> Result<T>;

    fn text(self) -> Result<String>;
}

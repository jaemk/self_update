use http::{HeaderValue, StatusCode};
use serde::de::DeserializeOwned;

use crate::Result;
pub use http::header;
pub use http::HeaderMap;

mod reqwest;
mod ureq;

#[cfg(feature = "reqwest")]
pub use reqwest::*;
#[cfg(feature = "ureq")]
pub use ureq::*;

pub trait HttpResponse {
    fn headers(&self) -> &HeaderMap<HeaderValue>;

    fn status(&self) -> StatusCode;

    fn body(self) -> impl std::io::Read;

    fn json<T: DeserializeOwned>(self) -> Result<T>;

    fn text(self) -> Result<String>;
}

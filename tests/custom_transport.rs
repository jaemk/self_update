//! A custom HTTP transport, implemented and injected entirely through the crate's public API,
//! drives a backend with no reqwest/ureq and no network. This is a downstream-perspective
//! regression test: it compiles only if `self_update::http_client::{HttpClient, HttpResponse}`
//! are publicly nameable and the builder's `http_client` setter accepts an `Arc<dyn HttpClient>`.
//! An in-crate test cannot catch a private `http_client` module, since it has crate-internal access.

#![cfg(feature = "github")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use self_update::http_client::{HeaderMap, HttpClient, HttpResponse};

/// A response backed by a canned body. Implementing `HttpResponse` requires naming
/// `serde_json::Value` (the JSON currency of `json_value`), so a custom-transport author depends
/// on `serde_json`.
struct CannedResponse {
    body: String,
    headers: HeaderMap,
}

impl HttpResponse for CannedResponse {
    fn headers(&self) -> &HeaderMap {
        &self.headers
    }
    fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
        Box::new(std::io::Cursor::new(self.body.into_bytes()))
    }
}

/// A transport that records each requested URL and answers with a canned body.
struct CannedClient {
    body: String,
    requested: Arc<Mutex<Vec<String>>>,
}

impl HttpClient for CannedClient {
    fn get(
        &self,
        url: &str,
        _headers: &HeaderMap,
        _timeout: Option<Duration>,
    ) -> self_update::Result<Box<dyn HttpResponse>> {
        self.requested.lock().unwrap().push(url.to_string());
        Ok(Box::new(CannedResponse {
            body: self.body.clone(),
            headers: HeaderMap::new(),
        }))
    }
}

#[test]
fn custom_transport_injected_through_public_api_drives_a_backend() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("o")
        .repo_name("r")
        .http_client(Arc::new(CannedClient {
            body: r#"[{"tag_name":"v4.5.6","created_at":"2020-01-01T00:00:00Z","name":"v4.5.6","assets":[]}]"#.to_string(),
            requested: requested.clone(),
        }))
        .build()
        .unwrap()
        .fetch()
        .unwrap();

    let releases = releases.into_vec();
    assert_eq!(releases.len(), 1, "the backend parsed the canned response");
    assert_eq!(releases[0].version(), "4.5.6");

    let urls = requested.lock().unwrap();
    assert_eq!(
        urls.len(),
        1,
        "exactly one request went through the transport"
    );
    assert!(
        urls[0].contains("/repos/o/r/releases"),
        "the transport saw the URL the backend asked for, got {:?}",
        urls[0]
    );
}

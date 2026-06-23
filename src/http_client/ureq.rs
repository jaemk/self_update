#![cfg(feature = "ureq")]

use ureq::tls::TlsProvider;
use ureq::{Agent, Body, http::Response};

use super::{ClientOverride, HeaderMap, HttpResponse};
use crate::{Error, Result, errors::status_to_error};

pub fn get(
    url: &str,
    headers: HeaderMap,
    timeout: Option<std::time::Duration>,
    client: &ClientOverride,
) -> Result<impl HttpResponse> {
    // Use the injected agent if present (by reference — `Agent::get` takes `&self`), otherwise
    // build one per call. An injected agent owns its own timeout/TLS/proxy config, so the
    // per-request `timeout` is only applied to the per-call agent (see the `### Custom HTTP client`
    // docs).
    let built_agent;
    let (agent, is_injected): (&Agent, bool) = match &client.agent {
        Some(agent) => (agent, true),
        None => {
            #[cfg(feature = "rustls")]
            let provider = TlsProvider::Rustls;
            #[cfg(not(feature = "rustls"))]
            let provider = TlsProvider::NativeTls;

            let config = Agent::config_builder()
                .tls_config(ureq::tls::TlsConfig::builder().provider(provider).build())
                .timeout_global(timeout)
                // Honor HTTP(S)_PROXY / NO_PROXY env vars (reqwest does this automatically).
                .proxy(ureq::Proxy::try_from_env())
                // Disable ureq's built-in status-error so we reach our own is_success() check,
                // which maps the status to the structured NotFound/Unauthorized/HttpStatus variants.
                .http_status_as_error(false)
                .build();
            built_agent = Agent::new_with_config(config);
            (&built_agent, false)
        }
    };
    let mut req = agent.get(url);

    for (key, value) in headers.into_iter() {
        if let Some(key) = key {
            req = req.header(key, value);
        }
    }

    let res = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) if is_injected => {
            // An injected agent has http_status_as_error=true (the ureq default) and cannot be
            // reconfigured. When it fires StatusCode, extract the code and map to structured error.
            return Err(status_to_error(code, url));
        }
        Err(e) => return Err(Error::Transport(Box::new(e))),
    };

    if !res.status().is_success() {
        return Err(status_to_error(res.status().as_u16(), url));
    }

    Ok(res)
}

impl HttpResponse for Response<Body> {
    fn headers(&self) -> &HeaderMap<http::HeaderValue> {
        Response::headers(self)
    }

    fn body(self) -> impl std::io::Read {
        self.into_body().into_reader()
    }

    fn json<T: serde::de::DeserializeOwned>(mut self) -> Result<T> {
        Ok(self.body_mut().read_json::<T>()?)
    }

    fn text(mut self) -> Result<String> {
        Ok(self.body_mut().read_to_string()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_client::ClientOverride;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// Serve a single HTTP response (the given status line + a short body) over a fresh loopback
    /// listener, then close. Returns the base URL (`http://127.0.0.1:<port>/`). No external network.
    fn stub(status: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "err";
                let out = format!(
                    "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes());
                let _ = stream.flush();
            }
        });
        base
    }

    /// Default-agent path (`is_injected == false`): the per-call agent is built with
    /// `http_status_as_error(false)`, so a non-2xx response is returned to our own `is_success()`
    /// check at the bottom of `get`, which routes it through `status_to_error`.
    fn get_default(status: &'static str) -> Error {
        let client = ClientOverride::default();
        let base = stub(status);
        super::get(&base, HeaderMap::new(), None, &client)
            .err()
            .expect("non-2xx must be an Err")
    }

    /// Injected-agent path (`is_injected == true`): a user-supplied default `ureq::Agent` has
    /// `http_status_as_error == true` (the ureq default) and fires `ureq::Error::StatusCode(code)`
    /// on a non-2xx. That hits the `Err(ureq::Error::StatusCode(code)) if is_injected` arm, which
    /// extracts the code and maps it through `status_to_error`. This is the arm the implementor
    /// flagged as having no isolated unit test.
    fn get_injected(status: &'static str) -> Error {
        let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
        let client = ClientOverride { agent: Some(agent) };
        let base = stub(status);
        super::get(&base, HeaderMap::new(), None, &client)
            .err()
            .expect("non-2xx must be an Err")
    }

    /// Injected agent built with `http_status_as_error(false)` (the OTHER injected case): the user
    /// disabled ureq's status-error, so `call()` returns `Ok(res)` even on a non-2xx. The
    /// `StatusCode` arm is therefore NOT taken; instead control falls through to the bottom-of-`get`
    /// `!res.status().is_success()` check, which routes the status through `status_to_error`. This
    /// is the injected-agent path the implementor flagged as untested.
    fn get_injected_no_status_error(status: &'static str) -> Error {
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let client = ClientOverride { agent: Some(agent) };
        let base = stub(status);
        super::get(&base, HeaderMap::new(), None, &client)
            .err()
            .expect("non-2xx must be an Err")
    }

    #[test]
    fn injected_agent_no_status_error_falls_through_to_is_success_check() {
        // 404 must still map to NotFound via the bottom-of-`get` is_success() path (not the
        // StatusCode arm, which never fires when http_status_as_error(false)).
        let err = get_injected_no_status_error("404 Not Found");
        assert!(
            matches!(err, Error::NotFound { .. }),
            "injected no-status-error 404 must map to Error::NotFound via is_success(), got {:?}",
            err
        );
        assert_eq!(err.http_status(), Some(404));

        // 500 maps to HttpStatus carrying its exact code through the same fall-through path.
        let err = get_injected_no_status_error("500 Internal Server Error");
        assert!(
            matches!(err, Error::HttpStatus { status: 500, .. }),
            "injected no-status-error 500 must map to Error::HttpStatus(500), got {:?}",
            err
        );
        assert_eq!(err.http_status(), Some(500));
    }

    #[test]
    fn injected_agent_status_code_arm_maps_404_to_not_found() {
        let err = get_injected("404 Not Found");
        assert!(
            matches!(err, Error::NotFound { .. }),
            "injected-agent 404 must map to Error::NotFound, got {:?}",
            err
        );
        assert_eq!(err.http_status(), Some(404));
    }

    #[test]
    fn injected_agent_status_code_arm_maps_401_and_403_to_unauthorized() {
        let err = get_injected("401 Unauthorized");
        assert!(
            matches!(err, Error::Unauthorized { status: 401, .. }),
            "injected-agent 401 must map to Error::Unauthorized(401), got {:?}",
            err
        );
        let err = get_injected("403 Forbidden");
        assert!(
            matches!(err, Error::Unauthorized { status: 403, .. }),
            "injected-agent 403 must map to Error::Unauthorized(403), got {:?}",
            err
        );
    }

    #[test]
    fn injected_agent_status_code_arm_maps_500_and_400_to_http_status() {
        let err = get_injected("500 Internal Server Error");
        assert!(
            matches!(err, Error::HttpStatus { status: 500, .. }),
            "injected-agent 500 must map to Error::HttpStatus(500), got {:?}",
            err
        );
        let err = get_injected("400 Bad Request");
        assert!(
            matches!(err, Error::HttpStatus { status: 400, .. }),
            "injected-agent 400 must map to Error::HttpStatus(400), got {:?}",
            err
        );
    }

    #[test]
    fn default_agent_path_maps_statuses_identically_to_injected() {
        // The default per-call agent (`http_status_as_error(false)`) reaches the bottom-of-`get`
        // `is_success()` check and must produce the SAME structured variants as the injected-agent
        // `StatusCode` arm, so both ureq lanes agree.
        assert!(matches!(
            get_default("404 Not Found"),
            Error::NotFound { .. }
        ));
        assert!(matches!(
            get_default("401 Unauthorized"),
            Error::Unauthorized { status: 401, .. }
        ));
        assert!(matches!(
            get_default("403 Forbidden"),
            Error::Unauthorized { status: 403, .. }
        ));
        assert!(matches!(
            get_default("503 Service Unavailable"),
            Error::HttpStatus { status: 503, .. }
        ));
    }

    #[test]
    fn transport_failure_maps_to_transport_variant() {
        // A connection refused to a closed port (no listener) cannot complete the request, so the
        // catch-all `Err(e) => Error::Transport` arm fires (default agent) -- NOT a status variant.
        // Bind+drop to obtain a port nothing is listening on.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{}/", addr);
        let client = ClientOverride::default();
        let err = super::get(&url, HeaderMap::new(), None, &client)
            .err()
            .expect("connection refused must be an Err");
        assert!(
            matches!(err, Error::Transport(_)),
            "a failed (uncompleted) request must map to Error::Transport, got {:?}",
            err
        );
        assert_eq!(err.http_status(), None, "Transport has no HTTP status code");
    }
}

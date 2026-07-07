#![cfg(feature = "ureq")]

use std::time::Duration;

use ureq::tls::TlsProvider;
use ureq::{Agent, Body, http::Response};

use super::{HeaderMap, HttpClient, HttpResponse};
use crate::{Error, Result, errors::status_to_error};

/// The certificate set a crate-built ureq agent trusts. `Vec<Certificate<'static>>` so the roots
/// outlive the per-call agent built from them.
#[cfg(any(not(feature = "reqwest"), test))]
type UreqRootCerts = std::sync::Arc<Vec<ureq::tls::Certificate<'static>>>;

/// How a [`UreqClient`] obtains the agent for each request.
enum UreqInner {
    /// Build a fresh per-call agent honoring the per-request timeout, the TLS feature, and proxy-env.
    Default,
    /// A user-injected agent (via `From<ureq::Agent>` / the `ureq_agent` setter) that owns its own
    /// timeout/TLS/proxy config, so the per-request timeout is *not* applied to it.
    Injected(Agent),
    /// Build a fresh per-call agent (like [`Default`](UreqInner::Default), so it still honors the
    /// per-request timeout and proxy-env) that trusts these custom root certificates.
    #[cfg(any(not(feature = "reqwest"), test))]
    Certs(UreqRootCerts),
}

/// Sync [`HttpClient`] backed by a `ureq::Agent`.
pub struct UreqClient(UreqInner);

impl Default for UreqClient {
    fn default() -> Self {
        Self(UreqInner::Default)
    }
}

impl From<Agent> for UreqClient {
    fn from(agent: Agent) -> Self {
        Self(UreqInner::Injected(agent))
    }
}

/// Build a per-call ureq agent honoring the per-request `timeout`, the TLS feature, and proxy-env.
/// `root_certs`, when `Some`, replaces the default trust store with the supplied certificates.
fn build_call_agent(
    timeout: Option<Duration>,
    #[cfg(any(not(feature = "reqwest"), test))] root_certs: Option<UreqRootCerts>,
) -> Agent {
    use ureq::tls::TlsConfig;
    // When both TLS features are enabled, rustls wins (it is the crate default); otherwise fall
    // back to native-tls (also the case when no TLS feature is set).
    #[cfg(feature = "rustls")]
    let provider = TlsProvider::Rustls;
    #[cfg(not(feature = "rustls"))]
    let provider = TlsProvider::NativeTls;

    #[cfg(any(not(feature = "reqwest"), test))]
    let mut tls = TlsConfig::builder().provider(provider);
    #[cfg(all(feature = "reqwest", not(test)))]
    let tls = TlsConfig::builder().provider(provider);
    #[cfg(any(not(feature = "reqwest"), test))]
    if let Some(certs) = root_certs {
        tls = tls.root_certs(ureq::tls::RootCerts::Specific(certs));
    }
    let config = Agent::config_builder()
        .tls_config(tls.build())
        .timeout_global(timeout)
        // Honor HTTP(S)_PROXY / NO_PROXY env vars (reqwest does this automatically).
        .proxy(ureq::Proxy::try_from_env())
        // Disable ureq's built-in status-error so we reach our own is_success() check, which maps
        // the status to the structured NotFound/Unauthorized/HttpStatus variants.
        .http_status_as_error(false)
        .build();
    Agent::new_with_config(config)
}

// `client_with_root_certs` only dispatches to the ureq builder when reqwest is NOT also enabled
// (reqwest wins, exactly like `default_client`), so this is dead in a both-features lib build. Gate
// it to the lanes that actually reach it (and to `test`, where the ureq cert test exercises it).
#[cfg(any(not(feature = "reqwest"), test))]
impl UreqClient {
    /// Build a UreqClient that trusts the supplied custom root CA certificates.
    ///
    /// The certificates are parsed and validated here (a malformed PEM certificate returns `Err`);
    /// the agent itself is built per request in [`get`](HttpClient::get), so it still honors the
    /// per-request timeout and proxy-env. `RootCerts::Specific` replaces the default trust store, so
    /// only the supplied certificates are trusted (see the `add_root_certificate` docs).
    pub(crate) fn build_with_certs(
        certs: &[crate::tls::Certificate],
    ) -> std::result::Result<
        std::sync::Arc<dyn crate::http_client::HttpClient>,
        crate::http_client::ClientBuildError,
    > {
        let mut ureq_certs = Vec::with_capacity(certs.len());
        for cert in certs {
            let c = if cert.is_pem() {
                ureq::tls::Certificate::from_pem(cert.bytes())
                    .map_err(|e| format!("invalid PEM certificate: {e}"))?
            } else {
                // `from_der` is infallible in ureq; invalid DER bytes are surfaced at connection
                // time, not here (documented on `Certificate::from_der` / `add_root_certificate`).
                ureq::tls::Certificate::from_der(cert.bytes())
            };
            ureq_certs.push(c.to_owned());
        }
        Ok(std::sync::Arc::new(UreqClient(UreqInner::Certs(
            std::sync::Arc::new(ureq_certs),
        ))))
    }
}

impl HttpClient for UreqClient {
    fn get(
        &self,
        url: &str,
        headers: &HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<Box<dyn HttpResponse>> {
        // An injected agent owns its own timeout/TLS/proxy config, so the per-request `timeout` is
        // only applied to the crate-built (Default / Certs) agents.
        let built_agent;
        let (agent, is_injected): (&Agent, bool) = match &self.0 {
            UreqInner::Injected(agent) => (agent, true),
            UreqInner::Default => {
                built_agent = build_call_agent(
                    timeout,
                    #[cfg(any(not(feature = "reqwest"), test))]
                    None,
                );
                (&built_agent, false)
            }
            #[cfg(any(not(feature = "reqwest"), test))]
            UreqInner::Certs(certs) => {
                built_agent = build_call_agent(timeout, Some(certs.clone()));
                (&built_agent, false)
            }
        };
        let mut req = agent.get(url);

        for (key, value) in headers.iter() {
            req = req.header(key, value);
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

        Ok(Box::new(res))
    }
}

impl HttpResponse for Response<Body> {
    fn headers(&self) -> &HeaderMap<http::HeaderValue> {
        Response::headers(self)
    }

    fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
        Box::new((*self).into_body().into_reader())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let client = UreqClient::default();
        let base = stub(status);
        client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err")
    }

    /// Injected-agent path (`is_injected == true`): a user-supplied default `ureq::Agent` has
    /// `http_status_as_error == true` (the ureq default) and fires `ureq::Error::StatusCode(code)`
    /// on a non-2xx. That hits the `Err(ureq::Error::StatusCode(code)) if is_injected` arm, which
    /// extracts the code and maps it through `status_to_error`.
    fn get_injected(status: &'static str) -> Error {
        let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().build());
        let client = UreqClient::from(agent);
        let base = stub(status);
        client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err")
    }

    /// Injected agent built with `http_status_as_error(false)` (the OTHER injected case): the user
    /// disabled ureq's status-error, so `call()` returns `Ok(res)` even on a non-2xx. The
    /// `StatusCode` arm is therefore NOT taken; instead control falls through to the bottom-of-`get`
    /// `!res.status().is_success()` check, which routes the status through `status_to_error`.
    fn get_injected_no_status_error(status: &'static str) -> Error {
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let client = UreqClient::from(agent);
        let base = stub(status);
        client
            .get(&base, &HeaderMap::new(), None)
            .err()
            .expect("non-2xx must be an Err")
    }

    #[test]
    fn build_with_certs_rejects_non_pem() {
        // ureq's `tls::Certificate::from_pem` validates the PEM framing eagerly and errors when the
        // bytes contain no PEM certificate, so `build_with_certs` must surface a config-time `Err`
        // (the parse is deferred to here from the infallible `Certificate::from_pem` constructor)
        // rather than panicking or building an agent over garbage.
        let res = UreqClient::build_with_certs(&[crate::tls::Certificate::from_pem(
            b"not a pem certificate".to_vec(),
        )]);
        assert!(
            res.is_err(),
            "bytes with no PEM certificate must be rejected at build time, got Ok"
        );
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
        let client = UreqClient::default();
        let err = client
            .get(&url, &HeaderMap::new(), None)
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

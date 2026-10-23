//! The built-in client (the `fetch` feature): `ureq` with rustls, the
//! owner's one exception to the pure-Rust rule (answer 40), as textweaver's
//! fetcher uses it (`fetch.rs`, the owner's own code, relicensed under
//! answer 3), hardened for resuming.
//!
//! `https` only, at most five redirects, a 30-second connect limit, the
//! neutral User-Agent, an `Authorization` header that never follows a
//! redirect, and `Range` with `If-Range` to go on with a stopped download.
//! A `206` whose `Content-Range` starts anywhere but the byte asked for,
//! and a `416`, are "the server cannot resume"; a `200` to a range request
//! is the whole file again, which the downloader writes from the start.
//! The proxy variables of the environment are honored.

use std::time::Duration;

use ureq::Agent;
use ureq::config::RedirectAuthHeaders;
use ureq::http::Response;

use crate::USER_AGENT;
use crate::fetch::{FetchError, Fetched, Fetcher, Request};

/// The built-in HTTPS client.
#[derive(Clone, Debug)]
pub struct HttpFetcher {
    agent: Agent,
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpFetcher {
    /// The client for `https` addresses.
    pub fn new() -> Self {
        Self::build(true)
    }

    /// A client that also takes plain `http` and ignores the proxy
    /// variables, for the loopback server: tests only.
    #[cfg(test)]
    pub(crate) fn for_loopback() -> Self {
        Self::build(false)
    }

    fn build(https_only: bool) -> Self {
        let mut config = Agent::config_builder()
            .https_only(https_only)
            .http_status_as_error(false)
            .max_redirects(5)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            // Neutral on purpose: nothing about the owner or the computer
            // goes out with a request.
            .user_agent(USER_AGENT)
            .accept_encoding("identity")
            .redirect_auth_headers(RedirectAuthHeaders::Never);
        if !https_only {
            config = config.proxy(None);
        }
        Self {
            agent: config.build().into(),
        }
    }
}

/// The start byte of a `Content-Range: bytes START-END/TOTAL`.
fn range_start(value: &str) -> Option<u64> {
    value
        .trim()
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

fn header<B>(response: &Response<B>, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn transport_error(error: ureq::Error) -> FetchError {
    match error {
        ureq::Error::HostNotFound => FetchError::Resolve,
        ureq::Error::ConnectionFailed => FetchError::Connect,
        ureq::Error::Timeout(_) => FetchError::Timeout,
        ureq::Error::TooManyRedirects => FetchError::Redirect,
        ureq::Error::RequireHttpsOnly(_) => {
            FetchError::Refused("the address or a redirect is not https".to_owned())
        }
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::TlsRequired => FetchError::Tls,
        ureq::Error::Io(e) => match e.kind() {
            std::io::ErrorKind::ConnectionRefused => FetchError::Connect,
            std::io::ErrorKind::TimedOut => FetchError::Timeout,
            _ => FetchError::from_io(&e),
        },
        other => FetchError::Other(other.to_string()),
    }
}

impl Fetcher for HttpFetcher {
    fn open(&self, request: &Request<'_>) -> Result<Fetched, FetchError> {
        let mut call = self.agent.get(request.address);
        if let Some(token) = request.token {
            call = call.header("Authorization", format!("Bearer {}", token.expose()));
        }
        if request.from > 0 {
            call = call.header("Range", format!("bytes={}-", request.from));
            if let Some(validator) = request.if_range {
                call = call.header("If-Range", validator);
            }
        }
        let response = call.call().map_err(transport_error)?;
        let status = response.status().as_u16();
        let validator = header(&response, "etag").or_else(|| header(&response, "last-modified"));
        let start = match status {
            200 => 0,
            206 if request.from > 0 => {
                match header(&response, "content-range")
                    .as_deref()
                    .and_then(range_start)
                {
                    Some(start) if start == request.from => start,
                    _ => return Err(FetchError::CannotResume),
                }
            }
            206 => return Err(FetchError::CannotResume),
            404 | 410 => return Err(FetchError::Missing),
            416 => return Err(FetchError::CannotResume),
            other => return Err(FetchError::Http(other)),
        };
        if let Some(length) =
            header(&response, "content-length").and_then(|v| v.trim().parse::<u64>().ok())
            && start + length > request.limit
        {
            return Err(FetchError::TooLarge);
        }
        let reader = response.into_body().into_reader();
        Ok(Fetched {
            reader: Box::new(reader),
            start,
            validator,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_ranges_are_read() {
        assert_eq!(range_start("bytes 100-199/200"), Some(100));
        assert_eq!(range_start(" bytes 0-0/1"), Some(0));
        assert_eq!(range_start("bytes */200"), None);
        assert_eq!(range_start("items 1-2/3"), None);
    }
}

//! Getting bytes from a web address. [`Fetch`] is the seam: the downloader talks to it, tests give it
//! a fake, and the app gives it [`UreqFetch`].

use std::ffi::OsString;
use std::io::{self, Read};
use std::sync::Arc;
use std::time::Duration;

use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

/// How long a name lookup, a connection and a server's first answer may take. A stalled connection ends
/// on these, but the downloader does not wait for them: it abandons the request when cancelled (and
/// refuses a new download while that abandoned thread lives, so these bound that time).
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long one read of the body may wait for data. A stalled download ends (and keeps its partial file).
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// One answer to a request: the status line, the few headers the downloader looks at, and the body.
pub struct FetchResponse {
    pub status: u16,
    /// The `Location` header of a redirect.
    pub location: Option<String>,
    /// The `Content-Range` header of a partial answer, as sent (`bytes 100-199/1000`).
    pub content_range: Option<String>,
    /// The `Content-Length` header: the size of this body.
    pub content_length: Option<u64>,
    /// `Send`: the downloader reads it on a helper thread, so that a cancel does not wait for the network.
    pub body: Box<dyn Read + Send>,
}

impl std::fmt::Debug for FetchResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FetchResponse")
            .field("status", &self.status)
            .field("location", &self.location)
            .field("content_range", &self.content_range)
            .field("content_length", &self.content_length)
            .finish_non_exhaustive()
    }
}

/// An HTTPS `GET`. An implementation never follows a redirect: it returns the `3xx` answer and the
/// caller decides, so that every hop can be checked. `from` asks for the bytes from that offset on (a
/// `Range` header); the only other header an implementation may send is a generic `User-Agent`.
pub trait Fetch: Send + Sync {
    fn get(&self, url: &str, from: Option<u64>) -> io::Result<FetchResponse>;
}

/// What is said when a SOCKS proxy is set: the client is built without SOCKS, and would connect directly
/// (and look the name up itself) instead of through it.
const SOCKS_REFUSED: &str = "A SOCKS proxy is set (ALL_PROXY/HTTPS_PROXY), and Platter cannot download through it. Use an HTTP proxy or unset it.";

/// The variables the client reads a proxy from, highest priority first.
const PROXY_VARIABLES: [&str; 6] = [
    "ALL_PROXY",
    "all_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
];

/// The proxy variable that is set but that the client cannot read, if any.
///
/// The client takes the first variable that holds a proxy it can parse and skips the ones it cannot, so a
/// misspelt `ALL_PROXY` would let a lower one, or no proxy at all, win without a word. The variable the
/// user means is the first one that is set and not empty; when that one does not parse, this names it.
/// `get` reads one variable (the environment's, or a fake in a test); a value that is not Unicode cannot
/// be read either.
pub fn unreadable_proxy_variable(get: impl Fn(&str) -> Option<OsString>) -> Option<&'static str> {
    for name in PROXY_VARIABLES {
        let Some(value) = get(name) else {
            continue;
        };

        match value.to_str() {
            Some(text) if text.trim().is_empty() => continue,
            Some(text) if ureq::Proxy::new(text).is_ok() => return None,
            _ => return Some(name),
        }
    }

    None
}

/// What is said when a proxy variable is set that cannot be read: connecting directly instead would
/// bypass the proxy the user asked for.
fn proxy_unreadable(variable: &str) -> String {
    format!("A proxy is set ({variable}) that Platter cannot read. Fix it or unset it.")
}

/// [`Fetch`] on `ureq`: TLS by `rustls` with the `ring` provider, given to the client itself (nothing is
/// installed as the process default), and certificates checked by the system's own verifier.
///
/// The proxy is the one of the environment (`ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`, each also in lower
/// case, and `NO_PROXY`), read once and given to the client explicitly; the desktop's own proxy settings
/// (GNOME, KDE) are not read unless they are exported as those variables. An HTTP(S) proxy is used. A
/// SOCKS proxy cannot be (the client has no SOCKS support): every request to a host it would serve is
/// refused with a plain error, without connecting. So is every request while a proxy variable is set
/// that cannot be read at all (see [`unreadable_proxy_variable`]).
pub struct UreqFetch {
    agent: ureq::Agent,
    /// The SOCKS proxy that was set, if any.
    socks: Option<ureq::Proxy>,
    /// The proxy variable that is set but unreadable, if any: nothing is requested then.
    unreadable: Option<&'static str>,
}

impl UreqFetch {
    /// With the proxy of the environment.
    pub fn new() -> Self {
        let unreadable = unreadable_proxy_variable(|name| std::env::var_os(name));

        Self::with_environment(ureq::Proxy::try_from_env(), unreadable)
    }

    /// With `proxy` (`None`: a direct connection) instead of the environment's.
    pub fn with_proxy(proxy: Option<ureq::Proxy>) -> Self {
        Self::with_environment(proxy, None)
    }

    /// With `proxy` as above, and refusing every request when `unreadable` names a proxy variable.
    pub fn with_environment(proxy: Option<ureq::Proxy>, unreadable: Option<&'static str>) -> Self {
        // A SOCKS proxy is never given to the client: it cannot use it (and panics at a connection when
        // one that was not read from the environment is configured). What it would serve is refused by
        // `get`; a host that `NO_PROXY` excludes is connected to directly, as the client would.
        let (proxy, socks) = match proxy {
            Some(proxy) if is_socks(proxy.protocol()) => (None, Some(proxy)),
            other => (other, None),
        };

        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .root_certs(RootCerts::PlatformVerifier)
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();

        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .https_only(true)
            // The downloader follows redirects itself, one checked hop at a time.
            .max_redirects(0)
            // A 4xx or 5xx answer is an answer, not a transport error.
            .http_status_as_error(false)
            .user_agent("platter")
            // Nothing else about this machine goes out: no `Accept`, and no encodings are offered.
            .accept("")
            .accept_encoding("")
            .timeout_resolve(Some(RESOLVE_TIMEOUT))
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_response(Some(RESPONSE_TIMEOUT))
            .timeout_per_read(Some(READ_TIMEOUT))
            .proxy(proxy)
            .build()
            .new_agent();

        Self {
            agent,
            socks,
            unreadable,
        }
    }

    /// Whether `url` would go through a SOCKS proxy (one that `NO_PROXY` does not exclude it from).
    fn through_socks(&self, url: &str) -> bool {
        let Some(proxy) = &self.socks else {
            return false;
        };

        match url.parse::<Uri>() {
            Ok(uri) => !proxy.is_no_proxy(&uri),
            Err(_) => true,
        }
    }
}

/// Every protocol but the HTTP ones counts as SOCKS (the enum may grow): what is not known to work is
/// refused.
fn is_socks(protocol: ureq::ProxyProtocol) -> bool {
    !matches!(protocol, ureq::ProxyProtocol::Http | ureq::ProxyProtocol::Https)
}

impl Default for UreqFetch {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetch for UreqFetch {
    fn get(&self, url: &str, from: Option<u64>) -> io::Result<FetchResponse> {
        if let Some(variable) = self.unreadable {
            return Err(io::Error::other(proxy_unreadable(variable)));
        }

        if self.through_socks(url) {
            return Err(io::Error::other(SOCKS_REFUSED));
        }

        let mut request = self.agent.get(url);
        if let Some(from) = from {
            request = request.header("Range", format!("bytes={from}-"));
        }

        let response = request.call().map_err(ureq::Error::into_io)?;

        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        Ok(FetchResponse {
            status: response.status().as_u16(),
            location: header("location"),
            content_range: header("content-range"),
            content_length: header("content-length").and_then(|value| value.trim().parse().ok()),
            body: Box::new(response.into_body().into_reader()),
        })
    }
}

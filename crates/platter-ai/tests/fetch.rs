//! The network client and the proxy of the environment: an HTTP proxy is used, a SOCKS proxy (which the
//! client cannot speak) refuses the request instead of letting it bypass the proxy.

use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener};
use std::thread;
use std::time::{Duration, Instant};

use platter_ai::{Fetch, UreqFetch, unreadable_proxy_variable};
use ureq::Proxy;

const URL: &str = "https://models.example/tiny.gguf";

/// A server on this machine that notes what connects to it.
fn listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();

    (listener, address)
}

/// Whether anything connected within `wait`.
fn connected_within(listener: &TcpListener, wait: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < wait {
        if listener.accept().is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }

    false
}

#[test]
fn a_socks_proxy_refuses_the_request_without_connecting_to_anything() {
    for scheme in ["socks4", "socks4a", "socks5", "socks5h"] {
        let (listener, address) = listener();
        let proxy = Proxy::new(&format!("{scheme}://{address}")).unwrap();
        let fetch = UreqFetch::with_proxy(Some(proxy));

        let error = fetch.get(URL, None).unwrap_err();

        assert!(
            error.to_string().contains("SOCKS proxy is set"),
            "{scheme}: {error}"
        );
        assert!(
            !connected_within(&listener, Duration::from_millis(300)),
            "{scheme}: the proxy was connected to"
        );
    }
}

#[test]
fn a_socks_proxy_does_not_refuse_a_host_that_no_proxy_excludes() {
    let (listener, address) = listener();
    let proxy = Proxy::builder(ureq::ProxyProtocol::Socks5)
        .host("127.0.0.1")
        .port(address.port())
        .no_proxy("localhost")
        .build()
        .unwrap();
    let fetch = UreqFetch::with_proxy(Some(proxy));
    // A port nobody listens on, on this machine.
    let closed = {
        let (closed, address) = self::listener();
        drop(closed);
        format!("https://localhost:{}/tiny.gguf", address.port())
    };

    // Not refused for the proxy: it goes on to connect to the host itself, which fails on its own.
    let error = fetch.get(&closed, None).unwrap_err();

    assert!(!error.to_string().contains("SOCKS"), "{error}");
    assert!(!connected_within(&listener, Duration::from_millis(100)));
}

#[test]
fn an_http_proxy_is_used_for_the_connection() {
    let (listener, address) = listener();
    listener.set_nonblocking(false).unwrap();
    let proxy = Proxy::new(&format!("http://{address}")).unwrap();
    let fetch = UreqFetch::with_proxy(Some(proxy));

    let server = thread::spawn(move || {
        let (connection, _) = listener.accept().unwrap();
        let mut first = String::new();
        BufReader::new(connection).read_line(&mut first).unwrap();

        first
    });
    // The proxy hangs up on the tunnel request: the request fails, but it was made through the proxy.
    let error = fetch.get(URL, None).unwrap_err();

    let first = server.join().unwrap();
    assert!(
        first.starts_with("CONNECT models.example:443 "),
        "{first:?} ({error})"
    );
}

/// The environment of a test: only these variables are set.
fn environment<'a>(set: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        set.iter()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| OsString::from(value))
    }
}

#[test]
fn a_proxy_variable_that_cannot_be_read_refuses_the_request_without_connecting() {
    for (variable, value) in [
        ("ALL_PROXY", "bogus://127.0.0.1:1"),
        ("https_proxy", "socks4h://127.0.0.1:1"),
        ("HTTP_PROXY", "http://"),
        ("http_proxy", "http:// not a host"),
    ] {
        let (listener, address) = listener();
        let value = value.replace("127.0.0.1:1", &address.to_string());
        let unreadable = unreadable_proxy_variable(environment(&[(variable, &value)]));
        assert_eq!(unreadable, Some(variable), "{value}");
        let fetch = UreqFetch::with_environment(None, unreadable);

        let error = fetch.get(URL, None).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("A proxy is set ({variable}) that Platter cannot read. Fix it or unset it.")
        );
        assert!(
            !connected_within(&listener, Duration::from_millis(200)),
            "{value}: something was connected to"
        );
    }
}

#[test]
fn an_unreadable_variable_is_not_passed_over_for_a_lower_one() {
    let unreadable = unreadable_proxy_variable(environment(&[
        ("ALL_PROXY", "bogus://proxy.example:8080"),
        ("HTTPS_PROXY", "http://proxy.example:8080"),
    ]));

    assert_eq!(unreadable, Some("ALL_PROXY"));
}

#[test]
fn a_readable_empty_or_lower_priority_broken_variable_refuses_nothing() {
    let readable = environment(&[("HTTPS_PROXY", "http://proxy.example:8080")]);
    assert_eq!(unreadable_proxy_variable(readable), None);

    // Empty counts as unset, and an unreadable variable below a readable one is never reached.
    let empty_above = environment(&[("ALL_PROXY", ""), ("http_proxy", "http://proxy.example:8080")]);
    assert_eq!(unreadable_proxy_variable(empty_above), None);
    let broken_below = environment(&[
        ("all_proxy", "socks5://proxy.example:1080"),
        ("HTTP_PROXY", "bogus://x"),
    ]);
    assert_eq!(unreadable_proxy_variable(broken_below), None);

    assert_eq!(unreadable_proxy_variable(environment(&[])), None);
}

#[test]
fn a_readable_http_proxy_with_no_unreadable_variable_is_still_used() {
    let (listener, address) = listener();
    listener.set_nonblocking(false).unwrap();
    let value = format!("http://{address}");
    let unreadable = unreadable_proxy_variable(environment(&[("HTTP_PROXY", &value)]));
    assert_eq!(unreadable, None);
    let fetch = UreqFetch::with_environment(Some(Proxy::new(&value).unwrap()), unreadable);

    let server = thread::spawn(move || {
        let (connection, _) = listener.accept().unwrap();
        let mut first = String::new();
        BufReader::new(connection).read_line(&mut first).unwrap();

        first
    });
    let error = fetch.get(URL, None).unwrap_err();

    let first = server.join().unwrap();
    assert!(
        first.starts_with("CONNECT models.example:443 "),
        "{first:?} ({error})"
    );
}

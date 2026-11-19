//! HTTP access, behind a trait so tests and the mock never touch the network.

use std::io::{Read, Write};
use std::net::{Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, ErrorKind, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    /// Body bytes (capped by the client).
    pub body: Vec<u8>,
}

/// A response whose body is read as it arrives.
pub struct HttpStream {
    pub status: u16,
    /// `Content-Length`, when the server sent one.
    pub length: Option<u64>,
    pub body: Box<dyn Read + Send>,
}

/// Blocking HTTP GET.
pub trait HttpClient: Send + Sync {
    /// GETs `url` with extra headers. Non-2xx statuses are returned, not errors.
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse>;

    /// GETs `url` for a download too large to hold in memory or to fetch within [`get`]'s
    /// time limit: the body is read as it arrives, with no overall limit on size or time (each
    /// read still times out). The default reads the whole body through [`get`].
    ///
    /// [`get`]: HttpClient::get
    fn get_stream(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpStream> {
        let response = self.get(url, headers)?;
        Ok(HttpStream {
            status: response.status,
            length: Some(response.body.len() as u64),
            body: Box::new(std::io::Cursor::new(response.body)),
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoHttp;

impl HttpClient for NoHttp {
    fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResponse> {
        Err(Error::new(ErrorKind::HttpUnavailable).with("url", url))
    }
}

#[cfg(feature = "http")]
#[derive(Debug, Clone)]
pub struct UreqClient {
    agent: ureq::Agent,
    /// For [`HttpClient::get_stream`]: no overall timeout, since a large file on a slow line
    /// takes minutes; a stalled connection still fails at the per-read timeout.
    download: ureq::Agent,
}

#[cfg(feature = "http")]
const USER_AGENT: &str = concat!("colab-playground/", env!("CARGO_PKG_VERSION"));

#[cfg(feature = "http")]
impl Default for UreqClient {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build();
        let download = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build();
        UreqClient {
            agent: ureq::Agent::new_with_config(config),
            download: ureq::Agent::new_with_config(download),
        }
    }
}

#[cfg(feature = "http")]
impl HttpClient for UreqClient {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut resp = req.call().map_err(|e| {
            Error::new(ErrorKind::Http)
                .with("url", url)
                .detail(e.to_string())
        })?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| {
                Error::new(ErrorKind::Http)
                    .with("url", url)
                    .detail(e.to_string())
            })?;
        Ok(HttpResponse { status, body })
    }

    fn get_stream(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpStream> {
        let mut req = self.download.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req.call().map_err(|e| {
            Error::new(ErrorKind::Http)
                .with("url", url)
                .detail(e.to_string())
        })?;
        let status = resp.status().as_u16();
        let length = resp.body().content_length();
        Ok(HttpStream {
            status,
            length,
            // No size limit here: the caller counts what it reads and stops at its own.
            body: Box::new(resp.into_body().into_reader()),
        })
    }
}

pub fn default_client() -> Arc<dyn HttpClient> {
    #[cfg(feature = "http")]
    {
        Arc::new(UreqClient::default())
    }
    #[cfg(not(feature = "http"))]
    {
        Arc::new(NoHttp)
    }
}

/// GETs `http://127.0.0.1:<port><path>` and returns the status code.
pub fn local_http_status(port: u16, path: &str, timeout: Duration) -> Result<u16> {
    if !path.starts_with('/') || path.chars().any(|c| c.is_control() || c == ' ') {
        return Err(Error::invalid(
            "path",
            "health path must start with '/' and contain no spaces",
        ));
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nUser-Agent: colab-playground\r\n\r\n"
    );
    stream.write_all(request.as_bytes())?;
    let mut buf = [0u8; 256];
    let mut got = Vec::new();
    while !got.windows(2).any(|w| w == b"\r\n") && got.len() < 4096 {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    parse_status_line(&got)
}

pub fn local_port_open(port: u16, timeout: Duration) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), timeout).is_ok()
}

fn addr_bound(addr: SocketAddr) -> bool {
    match TcpListener::bind(addr) {
        Ok(listener) => {
            drop(listener);
            false
        }
        Err(e) => matches!(
            e.kind(),
            std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
        ),
    }
}

/// A forward is opened as `http://localhost:<port>`, and `localhost` can resolve to `[::1]`,
/// where another program may be listening while `127.0.0.1:<port>` is still free to bind.
/// A bind also finds a socket that is bound but not accepting. On Windows it does not collide
/// with a listener on the wildcard address, so the connects stay.
pub fn local_port_in_use(port: u16, timeout: Duration) -> bool {
    let v4 = SocketAddr::from(([127, 0, 0, 1], port));
    let v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
    addr_bound(v4)
        || addr_bound(v6)
        || local_port_open(port, timeout)
        || TcpStream::connect_timeout(&v6, timeout.min(Duration::from_millis(150))).is_ok()
}

fn parse_status_line(bytes: &[u8]) -> Result<u16> {
    let line = String::from_utf8_lossy(bytes);
    let first = line.lines().next().unwrap_or_default();
    let mut parts = first.split_whitespace();
    match (
        parts.next(),
        parts.next().and_then(|c| c.parse::<u16>().ok()),
    ) {
        (Some(v), Some(code)) if v.starts_with("HTTP/") => Ok(code),
        _ => Err(Error::new(ErrorKind::Parse).detail(format!("bad HTTP status line: {first:?}"))),
    }
}

/// Percent-encodes a query component (RFC 3986 unreserved characters kept).
pub fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_line_parsing() {
        assert_eq!(parse_status_line(b"HTTP/1.1 200 OK\r\n").unwrap(), 200);
        assert_eq!(parse_status_line(b"HTTP/1.0 503 Busy\r\n").unwrap(), 503);
        assert!(parse_status_line(b"garbage").is_err());
    }

    #[test]
    fn query_encoding() {
        assert_eq!(encode_query("qwen 3/gguf&x"), "qwen%203%2Fgguf%26x");
        assert_eq!(encode_query("a-b_c.d~"), "a-b_c.d~");
    }

    #[test]
    fn local_probe_against_a_tiny_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            // First connection: the port check (connects and closes).
            drop(listener.accept().unwrap());
            // Second connection: the health probe.
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = s.read(&mut buf).unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("GET /health HTTP/1.1\r\n"));
            s.write_all(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
        });
        assert!(local_port_open(port, Duration::from_secs(2)));
        assert_eq!(
            local_http_status(port, "/health", Duration::from_secs(5)).unwrap(),
            204
        );
        server.join().unwrap();
        assert!(local_http_status(port, "no-slash", Duration::from_secs(1)).is_err());
        assert!(
            NoHttp
                .get("https://x", &[])
                .unwrap_err()
                .is(ErrorKind::HttpUnavailable)
        );
    }

    #[test]
    fn local_port_in_use_detects_ipv4_and_ipv6_loopback() {
        let v4 = TcpListener::bind("127.0.0.1:0").unwrap();
        let v4_port = v4.local_addr().unwrap().port();
        assert!(local_port_in_use(v4_port, Duration::from_millis(200)));
        drop(v4);

        if let Ok(v6) = TcpListener::bind("[::1]:0") {
            let v6_port = v6.local_addr().unwrap().port();
            assert!(local_port_in_use(v6_port, Duration::from_millis(200)));
            drop(v6);
        }
    }
}

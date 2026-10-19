//! Minimal HTTP/1.1 for a page served on 127.0.0.1 and for calling the local
//! engine. No TLS, no keep-alive: every exchange is one request on one
//! connection, closed afterwards.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const MAX_HEADER_BYTES: usize = 32 * 1024;
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn query_param(&self, name: &str) -> Option<String> {
        self.query.split('&').find_map(|kv| {
            let mut it = kv.splitn(2, '=');
            let k = it.next()?;
            let v = it.next().unwrap_or("");
            if k == name {
                Some(percent_decode(v))
            } else {
                None
            }
        })
    }
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Reads one request. Rejects oversized headers and bodies.
pub fn read_request<R: Read>(stream: R) -> io::Result<Request> {
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    let mut header_bytes = 0usize;

    let n = r.read_line(&mut line)?;
    if n == 0 {
        return Err(bad("empty request"));
    }
    header_bytes += n;
    let mut parts = line.trim_end().split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let version = parts.next().unwrap_or("");
    if method.is_empty() || !target.starts_with('/') || !version.starts_with("HTTP/1.") {
        return Err(bad("bad request line"));
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };

    let mut headers = Vec::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            return Err(bad("headers cut short"));
        }
        header_bytes += n;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(bad("headers too large"));
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some((k, v)) = t.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    let mut req = Request {
        method,
        path,
        query,
        headers,
        body: Vec::new(),
    };
    if let Some(len) = req.header("content-length") {
        let len: usize = len.parse().map_err(|_| bad("bad content-length"))?;
        if len > MAX_BODY_BYTES {
            return Err(bad("body too large"));
        }
        let mut body = vec![0u8; len];
        r.read_exact(&mut body)?;
        req.body = body;
    } else if req
        .header("transfer-encoding")
        .map(|v| v.eq_ignore_ascii_case("chunked"))
        .unwrap_or(false)
    {
        return Err(bad("chunked request bodies are not accepted"));
    }
    Ok(req)
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(status: u16, v: &serde_json::Value) -> Self {
        Response {
            status,
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_vec(v).unwrap_or_default(),
        }
    }
    pub fn html(body: &str) -> Self {
        Response {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }
    pub fn error(status: u16, msg: &str) -> Self {
        Response::json(status, &serde_json::json!({ "ok": false, "error": msg }))
    }
}

pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

pub fn write_response<W: Write>(w: &mut W, resp: &Response) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; img-src 'self' data:\r\nConnection: close\r\n\r\n",
        resp.status,
        reason(resp.status),
        resp.content_type,
        resp.body.len()
    );
    w.write_all(head.as_bytes())?;
    w.write_all(&resp.body)?;
    w.flush()
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Answer from a plain-HTTP call.
#[derive(Debug, Clone)]
pub struct ClientResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ClientResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Splits `http://host:port/base` into (host, port, base path without trailing slash).
pub fn parse_http_url(url: &str) -> io::Result<(String, u16, String)> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| bad("only http:// engine addresses are supported"))?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().map_err(|_| bad("bad port"))?),
        None => (hostport.to_string(), 80),
    };
    if host.is_empty() {
        return Err(bad("missing host"));
    }
    Ok((host, port, path.trim_end_matches('/').to_string()))
}

/// One HTTP/1.1 call with `Connection: close`. Handles chunked replies.
pub fn call(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    timeout: Duration,
) -> io::Result<ClientResponse> {
    let (host, port, path) = parse_http_url(url)?;
    let path = if path.is_empty() { "/".to_string() } else { path };
    let addr = (host.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| bad("cannot resolve engine address"))?;
    let connect_timeout = timeout.min(Duration::from_secs(3));
    let mut s = TcpStream::connect_timeout(&addr, connect_timeout)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let body = body.unwrap_or(&[]);
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    if method != "GET" {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    if method != "GET" {
        s.write_all(body)?;
    }
    s.flush()?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    parse_client_response(&raw)
}

pub fn parse_client_response(raw: &[u8]) -> io::Result<ClientResponse> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| bad("reply has no header end"))?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("bad status line"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let mut resp = ClientResponse {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    };
    let chunked = resp
        .header("transfer-encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    if chunked {
        resp.body = dechunk(&resp.body)?;
    }
    Ok(resp)
}

pub fn dechunk(mut b: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| bad("chunk size line missing"))?;
        let size_txt = String::from_utf8_lossy(&b[..eol]);
        let size_txt = size_txt.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_txt, 16).map_err(|_| bad("bad chunk size"))?;
        b = &b[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if b.len() < size {
            return Err(bad("chunk cut short"));
        }
        out.extend_from_slice(&b[..size]);
        b = &b[size..];
        if b.starts_with(b"\r\n") {
            b = &b[2..];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request_with_body_and_query() {
        let raw = b"POST /api/tune?x=1&name=a%20b HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: 5\r\n\r\nhello";
        let r = read_request(&raw[..]).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/api/tune");
        assert_eq!(r.query_param("name").as_deref(), Some("a b"));
        assert_eq!(r.header("HOST"), Some("127.0.0.1:1"));
        assert_eq!(r.body, b"hello");
    }

    #[test]
    fn rejects_bad_request_line_and_huge_body() {
        assert!(read_request(&b"GARBAGE\r\n\r\n"[..]).is_err());
        let raw = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        );
        assert!(read_request(raw.as_bytes()).is_err());
    }

    #[test]
    fn dechunks() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;x=y\r\npedia\r\n0\r\n\r\n";
        let r = parse_client_response(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"Wikipedia");
    }

    #[test]
    fn parses_urls() {
        assert_eq!(
            parse_http_url("http://127.0.0.1:18006/").unwrap(),
            ("127.0.0.1".into(), 18006, "".into())
        );
        assert_eq!(
            parse_http_url("http://localhost/v1").unwrap(),
            ("localhost".into(), 80, "/v1".into())
        );
        assert!(parse_http_url("https://x").is_err());
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%2Fb+c"), "a/b c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}

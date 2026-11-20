use std::net::TcpStream as StdTcpStream;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use async_io::Async;
use futures_lite::{future, AsyncReadExt, AsyncWriteExt};
use futures_rustls::rustls;
use futures_rustls::{client::TlsStream, TlsConnector};

/// Low-level transport error. This module never logs anything itself —
/// every failure is returned to the caller as a value.
#[derive(Debug)]
pub enum HttpError {
    Connect(std::io::Error),
    Tls(std::io::Error),
    Io(std::io::Error),
    Timeout,
    BadResponse(String),
    Status(u16, Vec<u8>),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Connect(e) => write!(f, "failed to connect: {e}"),
            HttpError::Tls(e) => write!(f, "TLS error: {e}"),
            HttpError::Io(e) => write!(f, "I/O error: {e}"),
            HttpError::Timeout => write!(f, "request timed out"),
            HttpError::BadResponse(s) => write!(f, "unrecognized server response: {s}"),
            HttpError::Status(code, body) => {
                write!(f, "HTTP {code}: {}", String::from_utf8_lossy(body))
            }
        }
    }
}

impl std::error::Error for HttpError {}

pub struct Response {
    #[allow(dead_code)]
    pub status: u16,
    pub body: Vec<u8>,
}

/// What to send as the request body. `Json` keeps the original behavior;
/// `Multipart` is new, for endpoints that take a file (`sendPhoto`,
/// `sendDocument`, ...).
pub enum Body {
    Json(Vec<u8>),
    Multipart(MultipartForm),
}

/// A `multipart/form-data` body, built one field at a time.
///
/// Telegram accepts both plain fields (e.g. `chat_id`) and file fields in
/// the same form, so this holds either kind in the order they were added.
pub struct MultipartForm {
    boundary: String,
    parts: Vec<u8>,
}

impl MultipartForm {
    pub fn new() -> Self {
        // Random-enough and never legitimately part of a field value.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Self {
            boundary: format!("----tglite{nanos:x}"),
            parts: Vec::new(),
        }
    }

    /// Adds a plain text field, e.g. `chat_id` or `caption`.
    pub fn field(mut self, name: &str, value: impl AsRef<str>) -> Self {
        self.parts
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        self.parts.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        self.parts.extend_from_slice(value.as_ref().as_bytes());
        self.parts.extend_from_slice(b"\r\n");
        self
    }

    /// Adds a file field. `filename` is shown to Telegram/the user;
    /// `mime_type` defaults to `application/octet-stream` when `None`,
    /// which Telegram accepts for any file type.
    pub fn file(
        mut self,
        field_name: &str,
        filename: &str,
        mime_type: Option<&str>,
        bytes: &[u8],
    ) -> Self {
        let mime = mime_type.unwrap_or("application/octet-stream");
        self.parts
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        self.parts.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{field_name}\"; filename=\"{filename}\"\r\n"
            )
            .as_bytes(),
        );
        self.parts
            .extend_from_slice(format!("Content-Type: {mime}\r\n\r\n").as_bytes());
        self.parts.extend_from_slice(bytes);
        self.parts.extend_from_slice(b"\r\n");
        self
    }

    fn content_type_header(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }

    fn finish(mut self) -> Vec<u8> {
        self.parts
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        self.parts
    }
}

impl Default for MultipartForm {
    fn default() -> Self {
        Self::new()
    }
}

fn tls_connector() -> &'static TlsConnector {
    static CONNECTOR: OnceLock<TlsConnector> = OnceLock::new();
    CONNECTOR.get_or_init(|| {
        let root_store =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        TlsConnector::from(Arc::new(config))
    })
}

pub async fn request(
    host: &str,
    path: &str,
    body: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Response, HttpError> {
    request_with_body(host, path, body.map(Body::Json), timeout).await
}

/// Like [`request`], but accepts a [`Body`] so callers can send a
/// `multipart/form-data` request (for file uploads) as well as plain JSON.
pub async fn request_with_body(
    host: &str,
    path: &str,
    body: Option<Body>,
    timeout: Duration,
) -> Result<Response, HttpError> {
    let timeout_fut = async {
        async_io::Timer::after(timeout).await;
        Result::<Response, HttpError>::Err(HttpError::Timeout)
    };

    future::or(request_inner(host, path, body), timeout_fut).await
}

async fn request_inner(host: &str, path: &str, body: Option<Body>) -> Result<Response, HttpError> {
    let std_tcp = StdTcpStream::connect((host, 443)).map_err(HttpError::Connect)?;
    std_tcp.set_nonblocking(true).map_err(HttpError::Connect)?;

    let _ = std_tcp.set_nodelay(true);

    let tcp = Async::new(std_tcp).map_err(HttpError::Connect)?;

    let server_name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| {
        HttpError::Tls(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid DNS name",
        ))
    })?;

    let mut tls: TlsStream<Async<StdTcpStream>> = tls_connector()
        .connect(server_name, tcp)
        .await
        .map_err(HttpError::Tls)?;

    // Resolve the body (and its Content-Type) up front so the header block
    // below doesn't need to know which kind it is.
    let (content_type, payload): (Option<String>, Option<Vec<u8>>) = match body {
        Some(Body::Json(bytes)) => (Some("application/json".to_string()), Some(bytes)),
        Some(Body::Multipart(form)) => {
            let content_type = form.content_type_header();
            (Some(content_type), Some(form.finish()))
        }
        None => (None, None),
    };

    let method = if payload.is_some() { "POST" } else { "GET" };
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         User-Agent: Rust-Telegram-Bot/0.1\r\n\
         Connection: close\r\n"
    );
    if let Some(ref p) = payload {
        if let Some(ref ct) = content_type {
            request.push_str(&format!("Content-Type: {ct}\r\n"));
        }
        request.push_str(&format!("Content-Length: {}\r\n", p.len()));
    }
    request.push_str("\r\n");

    tls.write_all(request.as_bytes())
        .await
        .map_err(HttpError::Io)?;
    if let Some(p) = payload {
        tls.write_all(&p).await.map_err(HttpError::Io)?;
    }
    tls.flush().await.map_err(HttpError::Io)?;

    let mut raw = Vec::new();
    tls.read_to_end(&mut raw).await.map_err(HttpError::Io)?;

    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Result<Response, HttpError> {
    let header_end = find_subslice(raw, b"\r\n\r\n")
        .ok_or_else(|| HttpError::BadResponse("no end of headers found".into()))?;

    let header_text = std::str::from_utf8(&raw[..header_end])
        .map_err(|_| HttpError::BadResponse("headers are not valid UTF-8".into()))?;
    let mut lines = header_text.split("\r\n");

    let status_line = lines
        .next()
        .ok_or_else(|| HttpError::BadResponse("empty response".into()))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| HttpError::BadResponse(format!("malformed status line: {status_line}")))?;

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            match name.as_str() {
                "content-length" => content_length = value.parse().ok(),
                "transfer-encoding" if value.eq_ignore_ascii_case("chunked") => chunked = true,
                _ => {}
            }
        }
    }

    let body_start = header_end + 4;
    let raw_body = &raw[body_start..];

    let body = if chunked {
        decode_chunked(raw_body)?
    } else if let Some(len) = content_length {
        raw_body.get(..len).unwrap_or(raw_body).to_vec()
    } else {
        raw_body.to_vec()
    };

    if !(200..300).contains(&status) {
        return Err(HttpError::Status(status, body));
    }

    Ok(Response { status, body })
}

fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    let mut pos = 0;

    loop {
        let line_end = find_subslice(&data[pos..], b"\r\n")
            .ok_or_else(|| HttpError::BadResponse("malformed chunked stream".into()))?;
        let size_line = std::str::from_utf8(&data[pos..pos + line_end])
            .map_err(|_| HttpError::BadResponse("invalid chunk size".into()))?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| HttpError::BadResponse(format!("invalid chunk size: {size_hex}")))?;

        pos += line_end + 2;

        if size == 0 {
            break;
        }

        let chunk_end = pos + size;
        let chunk = data
            .get(pos..chunk_end)
            .ok_or_else(|| HttpError::BadResponse("chunk shorter than declared size".into()))?;
        out.extend_from_slice(chunk);

        pos = chunk_end + 2;
    }

    Ok(out)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

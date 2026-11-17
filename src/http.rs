use std::net::TcpStream as StdTcpStream;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use async_io::Async;
use futures_lite::{AsyncReadExt, AsyncWriteExt, future};
use futures_rustls::rustls;
use futures_rustls::{TlsConnector, client::TlsStream};

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
            HttpError::Connect(e) => write!(f, "не вдалося підключитись: {e}"),
            HttpError::Tls(e) => write!(f, "помилка TLS: {e}"),
            HttpError::Io(e) => write!(f, "помилка вводу-виводу: {e}"),
            HttpError::Timeout => write!(f, "таймаут запиту"),
            HttpError::BadResponse(s) => write!(f, "нерозпізнана відповідь сервера: {s}"),
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
    let timeout_fut = async {
        async_io::Timer::after(timeout).await;
        Result::<Response, HttpError>::Err(HttpError::Timeout)
    };

    future::or(request_inner(host, path, body), timeout_fut).await
}

async fn request_inner(
    host: &str,
    path: &str,
    body: Option<Vec<u8>>,
) -> Result<Response, HttpError> {
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

    let method = if body.is_some() { "POST" } else { "GET" };
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         User-Agent: Rust-Telegram-Bot/0.1\r\n\
         Connection: close\r\n"
    );
    if let Some(ref b) = body {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    request.push_str("\r\n");

    tls.write_all(request.as_bytes())
        .await
        .map_err(HttpError::Io)?;
    if let Some(b) = body {
        tls.write_all(&b).await.map_err(HttpError::Io)?;
    }
    tls.flush().await.map_err(HttpError::Io)?;

    let mut raw = Vec::new();
    tls.read_to_end(&mut raw).await.map_err(HttpError::Io)?;

    parse_response(&raw)
}

fn parse_response(raw: &[u8]) -> Result<Response, HttpError> {
    let header_end = find_subslice(raw, b"\r\n\r\n")
        .ok_or_else(|| HttpError::BadResponse("немає кінця заголовків".into()))?;

    let header_text = std::str::from_utf8(&raw[..header_end])
        .map_err(|_| HttpError::BadResponse("заголовки не є valid UTF-8".into()))?;
    let mut lines = header_text.split("\r\n");

    let status_line = lines
        .next()
        .ok_or_else(|| HttpError::BadResponse("порожня відповідь".into()))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| HttpError::BadResponse(format!("дивний статус-рядок: {status_line}")))?;

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
            .ok_or_else(|| HttpError::BadResponse("зламаний chunked stream".into()))?;
        let size_line = std::str::from_utf8(&data[pos..pos + line_end])
            .map_err(|_| HttpError::BadResponse("невалідний chunk-size".into()))?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| HttpError::BadResponse(format!("невалідний chunk-size: {size_hex}")))?;

        pos += line_end + 2;

        if size == 0 {
            break;
        }

        let chunk_end = pos + size;
        let chunk = data
            .get(pos..chunk_end)
            .ok_or_else(|| HttpError::BadResponse("chunk коротший за заявлений розмір".into()))?;
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

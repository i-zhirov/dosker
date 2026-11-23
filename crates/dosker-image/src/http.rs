//! A small blocking HTTP/1.1 client for the registry.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use sha2::{Digest, Sha256};
use url::Url;

/// The largest image asset the client accepts.
pub const MAX_ASSET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The largest site document the client accepts.
pub const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;

const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(120);
const USER_AGENT: &str = concat!("dosker/", env!("CARGO_PKG_VERSION"));

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

type ResponseHead = (u16, BTreeMap<String, String>, BufReader<Box<dyn ReadWrite>>);

trait ReadWrite: Read + Write {}

impl<T: Read + Write> ReadWrite for T {}

/// The result of one streaming download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadReport {
    /// The number of bytes written.
    pub bytes: u64,
    /// The `sha256:` digest of the bytes.
    pub digest: String,
}

/// An HTTP client failure.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// The URL cannot be parsed.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    /// The URL is not HTTPS and is not loopback HTTP.
    #[error("refusing a non-HTTPS URL: {0}")]
    Scheme(String),

    /// The redirect chain is too long.
    #[error("too many redirects")]
    Redirect,

    /// The response status is not 200.
    #[error("HTTP status {0}")]
    Status(u16),

    /// The response is larger than the limit.
    #[error("the response exceeds the {0} byte limit")]
    TooLarge(u64),

    /// The connection or read timed out.
    #[error("the connection timed out")]
    Timeout,

    /// The response is malformed.
    #[error("invalid HTTP response: {0}")]
    Protocol(String),

    /// The TLS handshake or certificate check failed.
    #[error("TLS error: {0}")]
    Tls(String),

    /// A filesystem or socket operation failed.
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
}

/// Fetches one document.
pub fn get(url: &str, max_bytes: u64) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();
    fetch(url, max_bytes, &mut |chunk| {
        body.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(body)
}

/// Streams one asset to a file and returns its digest.
pub fn download(
    url: &str,
    max_bytes: u64,
    destination: &Path,
) -> Result<DownloadReport, HttpError> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".dosker-download-{}-{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        let mut hasher = Sha256::new();
        let bytes = fetch(url, max_bytes, &mut |chunk| {
            file.write_all(chunk).map_err(HttpError::Io)?;
            hasher.update(chunk);
            Ok(())
        })?;
        file.flush()?;
        fs::rename(&temp, destination)?;
        Ok(DownloadReport {
            bytes,
            digest: format!("sha256:{}", hex(&hasher.finalize())),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Resolves one redirect and applies the scheme rule to the target.
pub fn redirect_target(origin: &Url, location: &str) -> Result<Url, HttpError> {
    let target = origin
        .join(location)
        .map_err(|_| HttpError::InvalidUrl(location.to_owned()))?;
    let allowed = if origin.scheme() == "https" {
        target.scheme() == "https"
    } else {
        target.scheme() == "http" && target.host_str().is_some_and(is_loopback)
    };
    if allowed {
        Ok(target)
    } else {
        Err(HttpError::Scheme(target.as_str().to_owned()))
    }
}

fn fetch(
    url: &str,
    max_bytes: u64,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), HttpError>,
) -> Result<u64, HttpError> {
    let mut current = Url::parse(url).map_err(|_| HttpError::InvalidUrl(url.to_owned()))?;
    for _ in 0..=MAX_REDIRECTS {
        ensure_scheme(&current)?;
        let (status, headers, reader) = request(&current)?;
        if is_redirect(status) {
            let location = headers
                .get("location")
                .ok_or_else(|| HttpError::Protocol("a redirect has no Location".to_owned()))?;
            current = redirect_target(&current, location)?;
            continue;
        }
        if status != 200 {
            return Err(HttpError::Status(status));
        }
        return read_body(reader, &headers, max_bytes, sink);
    }
    Err(HttpError::Redirect)
}

fn ensure_scheme(url: &Url) -> Result<(), HttpError> {
    let allowed = match url.scheme() {
        "https" => true,
        "http" => url.host_str().is_some_and(is_loopback),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(HttpError::Scheme(url.as_str().to_owned()))
    }
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn request(url: &Url) -> Result<ResponseHead, HttpError> {
    let mut stream = connect(url)?;
    let host = url
        .host_str()
        .ok_or_else(|| HttpError::InvalidUrl(url.as_str().to_owned()))?;
    let host_header = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let mut path = if url.path().is_empty() {
        "/".to_owned()
    } else {
        url.path().to_owned()
    };
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: {USER_AGENT}\r\nAccept: application/json, application/octet-stream\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).map_err(map_io)?;
    stream.flush().map_err(map_io)?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).map_err(map_io)?;
    let status = parse_status(&status_line)?;

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).map_err(map_io)?;
        if read == 0 {
            return Err(HttpError::Protocol("truncated headers".to_owned()));
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    Ok((status, headers, reader))
}

fn connect(url: &Url) -> Result<Box<dyn ReadWrite>, HttpError> {
    let host = url
        .host_str()
        .ok_or_else(|| HttpError::InvalidUrl(url.as_str().to_owned()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| HttpError::InvalidUrl(url.as_str().to_owned()))?;
    let address = (host, port)
        .to_socket_addrs()
        .map_err(map_io)?
        .next()
        .ok_or_else(|| HttpError::InvalidUrl(url.as_str().to_owned()))?;
    let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(map_io)?;
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(map_io)?;
    stream
        .set_write_timeout(Some(READ_TIMEOUT))
        .map_err(map_io)?;
    if url.scheme() == "https" {
        tls(stream, host)
    } else {
        Ok(Box::new(stream))
    }
}

fn tls(stream: TcpStream, host: &str) -> Result<Box<dyn ReadWrite>, HttpError> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| HttpError::Tls(error.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name =
        ServerName::try_from(host.to_owned()).map_err(|error| HttpError::Tls(error.to_string()))?;
    let connection = ClientConnection::new(Arc::new(config), name)
        .map_err(|error| HttpError::Tls(error.to_string()))?;
    Ok(Box::new(StreamOwned::new(connection, stream)))
}

fn parse_status(line: &str) -> Result<u16, HttpError> {
    line.split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| HttpError::Protocol(format!("invalid status line: {line:?}")))
}

fn read_body(
    mut reader: BufReader<Box<dyn ReadWrite>>,
    headers: &BTreeMap<String, String>,
    max_bytes: u64,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), HttpError>,
) -> Result<u64, HttpError> {
    if let Some(value) = headers.get("content-length") {
        let length: u64 = value
            .parse()
            .map_err(|_| HttpError::Protocol("invalid Content-Length".to_owned()))?;
        if length > max_bytes {
            return Err(HttpError::TooLarge(max_bytes));
        }
    }
    let chunked = headers.get("transfer-encoding").is_some_and(|value| {
        value
            .split(',')
            .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
    });

    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    if chunked {
        loop {
            let size = read_chunk_size(&mut reader)?;
            if size == 0 {
                break;
            }
            if total + size > max_bytes {
                return Err(HttpError::TooLarge(max_bytes));
            }
            let mut remaining = size;
            while remaining > 0 {
                let limit = buffer.len().min(remaining as usize);
                let read = reader.read(&mut buffer[..limit]).map_err(map_io)?;
                if read == 0 {
                    return Err(HttpError::Protocol("truncated chunk".to_owned()));
                }
                sink(&buffer[..read])?;
                total += read as u64;
                remaining -= read as u64;
            }
            let mut crlf = String::new();
            reader.read_line(&mut crlf).map_err(map_io)?;
        }
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).map_err(map_io)?;
            if read == 0 || line.trim_end_matches(['\r', '\n']).is_empty() {
                break;
            }
        }
    } else {
        loop {
            let read = reader.read(&mut buffer).map_err(map_io)?;
            if read == 0 {
                break;
            }
            total += read as u64;
            if total > max_bytes {
                return Err(HttpError::TooLarge(max_bytes));
            }
            sink(&buffer[..read])?;
        }
    }
    Ok(total)
}

fn read_chunk_size(reader: &mut BufReader<Box<dyn ReadWrite>>) -> Result<u64, HttpError> {
    let mut line = String::new();
    reader.read_line(&mut line).map_err(map_io)?;
    let size_text = line
        .trim_end_matches(['\r', '\n'])
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    u64::from_str_radix(size_text, 16)
        .map_err(|_| HttpError::Protocol("invalid chunk size".to_owned()))
}

fn map_io(error: io::Error) -> HttpError {
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        HttpError::Timeout
    } else {
        HttpError::Io(error)
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

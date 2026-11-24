//! A fixture registry site and a loopback static server for client tests.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const POLL: Duration = Duration::from_millis(5);

/// One image in a fixture site.
pub struct SiteImage {
    pub owner: String,
    pub name: String,
    pub version: String,
    pub tag: String,
    pub description: String,
    pub homepage: Option<String>,
    pub deprecated: Option<String>,
    pub license_summary: String,
    pub manifest_bytes: Vec<u8>,
    pub config_bytes: Vec<u8>,
    pub asset_tar: Vec<u8>,
    pub published_at: String,
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string(value).map_err(io::Error::other)?;
    fs::write(path, text + "\n")
}

/// Writes a catalog, the per-image indexes, the version documents, and the asset tars.
pub fn build_site(
    destination: &Path,
    base_url: &str,
    generated_at: &str,
    images: &[SiteImage],
) -> io::Result<()> {
    let base = base_url.trim_end_matches('/');
    let mut catalog = Vec::new();

    for image in images {
        let manifest_digest = sha256_digest(&image.manifest_bytes);
        let config_digest = sha256_digest(&image.config_bytes);
        let asset_digest = sha256_digest(&image.asset_tar);
        let hex_digest = manifest_digest.trim_start_matches("sha256:").to_owned();

        let assets = destination.join("assets");
        fs::create_dir_all(&assets)?;
        fs::write(assets.join(format!("{asset_digest}.tar")), &image.asset_tar)?;

        let version_dir = destination
            .join("v1")
            .join(&image.owner)
            .join(&image.name)
            .join("versions")
            .join(format!("sha256-{hex_digest}"));
        fs::create_dir_all(&version_dir)?;
        fs::write(version_dir.join("manifest.json"), &image.manifest_bytes)?;
        fs::write(version_dir.join("config.json"), &image.config_bytes)?;

        let prefix = format!(
            "{base}/v1/{}/{}/versions/sha256-{hex_digest}",
            image.owner, image.name
        );
        let record = json!({
            "version": image.version.clone(),
            "digest": manifest_digest.clone(),
            "config_digest": config_digest,
            "manifest_url": format!("{prefix}/manifest.json"),
            "config_url": format!("{prefix}/config.json"),
            "asset_url": format!("{base}/assets/{asset_digest}.tar"),
            "asset_digest": asset_digest,
            "asset_size": image.asset_tar.len() as u64,
            "published_at": image.published_at,
            "license_summary": image.license_summary,
            "licenses_url": format!("{prefix}/licenses.json"),
        });

        let mut tags = Map::new();
        tags.insert(image.tag.clone(), json!(manifest_digest));
        let mut index = json!({
            "schema": 1,
            "image": format!("{}/{}", image.owner, image.name),
            "namespace": image.owner,
            "name": image.name,
            "description": image.description,
            "tags": tags.clone(),
            "versions": [record],
        });
        let mut entry = json!({
            "namespace": image.owner,
            "name": image.name,
            "description": image.description,
            "tags": tags,
            "license_summary": image.license_summary,
            "requires_acceptance": image.license_summary != "redistributable",
            "latest": {"version": image.version, "digest": manifest_digest},
        });
        if let Some(homepage) = &image.homepage {
            index["homepage"] = json!(homepage);
            entry["homepage"] = json!(homepage);
        }
        if let Some(deprecated) = &image.deprecated {
            index["deprecated"] = json!(deprecated);
            entry["deprecated"] = json!(deprecated);
        }
        write_json(
            &destination
                .join("v1")
                .join(&image.owner)
                .join(&image.name)
                .join("index.json"),
            &index,
        )?;
        catalog.push(entry);
    }

    let document = json!({
        "schema": 1,
        "generated_at": generated_at,
        "registry": base,
        "images": catalog,
    });
    write_json(&destination.join("index.json"), &document)
}

/// A loopback HTTP server for a fixture site.
pub struct StaticServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    ranges: Arc<Mutex<Vec<Option<String>>>>,
    fail_next_asset: Arc<Mutex<Option<usize>>>,
    thread: Option<JoinHandle<()>>,
}

impl StaticServer {
    /// Starts a server on an ephemeral loopback port.
    pub fn start(root: PathBuf) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let fail_next_asset = Arc::new(Mutex::new(None));
        let thread = {
            let stop = Arc::clone(&stop);
            let requests = Arc::clone(&requests);
            let ranges = Arc::clone(&ranges);
            let fail_next_asset = Arc::clone(&fail_next_asset);
            let root = root.clone();
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = handle(stream, &root, &requests, &ranges, &fail_next_asset);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(POLL);
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Ok(Self {
            addr,
            stop,
            requests,
            ranges,
            fail_next_asset,
            thread: Some(thread),
        })
    }

    /// Returns the base URL, without a trailing slash.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Returns the request paths, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .map(|log| log.clone())
            .unwrap_or_default()
    }

    /// Returns the Range header of each request, in order.
    pub fn ranges(&self) -> Vec<Option<String>> {
        self.ranges
            .lock()
            .map(|log| log.clone())
            .unwrap_or_default()
    }

    /// Truncates the next asset response after this many bytes.
    pub fn fail_next_asset(&self, bytes: usize) {
        if let Ok(mut value) = self.fail_next_asset.lock() {
            *value = Some(bytes);
        }
    }

    /// Clears the request log.
    pub fn reset_requests(&self) {
        if let Ok(mut log) = self.requests.lock() {
            log.clear();
        }
        if let Ok(mut log) = self.ranges.lock() {
            log.clear();
        }
    }
}

impl Drop for StaticServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle(
    mut stream: TcpStream,
    root: &Path,
    requests: &Mutex<Vec<String>>,
    ranges: &Mutex<Vec<Option<String>>>,
    fail_next_asset: &Mutex<Option<usize>>,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") || buffer.len() > 8192 {
            break;
        }
    }

    let request = String::from_utf8_lossy(&buffer);
    let line = request.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    let range = request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("range")
            .then(|| value.trim().to_owned())
    });
    if let Ok(mut log) = requests.lock() {
        log.push(path.to_owned());
    }
    if let Ok(mut log) = ranges.lock() {
        log.push(range.clone());
    }
    if method != "GET" {
        return respond(&mut stream, 405, b"method not allowed\n", "text/plain");
    }

    let decoded = path.split('?').next().unwrap_or("/");
    let safe = !decoded.split('/').any(|segment| segment == "..");
    if !safe {
        return respond(&mut stream, 404, b"not found\n", "text/plain");
    }
    let file = root.join(decoded.trim_start_matches('/'));
    if !file.is_file() {
        return respond(&mut stream, 404, b"not found\n", "text/plain");
    }
    let bytes = fs::read(&file)?;
    let content_type = if file.extension().and_then(|value| value.to_str()) == Some("json") {
        "application/json"
    } else {
        "application/octet-stream"
    };
    let is_asset = file.extension().and_then(|value| value.to_str()) == Some("tar");
    if is_asset {
        let limit = fail_next_asset
            .lock()
            .ok()
            .and_then(|mut value| value.take());
        if let Some(limit) = limit {
            return respond_truncated(&mut stream, &bytes, limit);
        }
    }
    if let Some(start) = range.as_deref().and_then(range_start) {
        let total = bytes.len() as u64;
        if start >= total {
            return respond_range(
                &mut stream,
                416,
                "Range Not Satisfiable",
                &format!("bytes */{total}"),
                &[],
                content_type,
            );
        }
        return respond_range(
            &mut stream,
            206,
            "Partial Content",
            &format!("bytes {start}-{}/{total}", total - 1),
            &bytes[start as usize..],
            content_type,
        );
    }
    respond(&mut stream, 200, &bytes, content_type)
}

fn range_start(value: &str) -> Option<u64> {
    let rest = value.strip_prefix("bytes=")?;
    let (start, end) = rest.split_once('-')?;
    if !end.is_empty() {
        return None;
    }
    start.parse().ok()
}

fn respond(stream: &mut TcpStream, status: u16, body: &[u8], content_type: &str) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn respond_range(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_range: &str,
    body: &[u8],
    content_type: &str,
) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Range: {content_range}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn respond_truncated(stream: &mut TcpStream, body: &[u8], limit: usize) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&body[..body.len().min(limit)])?;
    stream.flush()
}

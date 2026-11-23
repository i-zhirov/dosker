use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::thread::{self, JoinHandle};

use dosker_image::http::{self, HttpError, MAX_DOCUMENT_BYTES};
use url::Url;

fn respond(responses: Vec<Vec<u8>>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let addr = listener.local_addr().expect("local address");
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            stream.write_all(&response).expect("write response");
            stream.flush().expect("flush response");
        }
    });
    (format!("http://{addr}"), handle)
}

fn response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\n{headers}Content-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

#[test]
fn a_loopback_document_is_fetched() {
    let (url, handle) = respond(vec![response(
        "200 OK",
        "Content-Type: application/json\r\n",
        b"{\"ok\":true}",
    )]);
    let body = http::get(&format!("{url}/index.json"), MAX_DOCUMENT_BYTES).expect("get");
    handle.join().expect("server");
    assert_eq!(body, b"{\"ok\":true}");
}

#[test]
fn a_remote_plain_http_url_is_refused() {
    let error =
        http::get("http://example.com/index.json", MAX_DOCUMENT_BYTES).expect_err("refused");
    assert!(matches!(error, HttpError::Scheme(_)));
}

#[test]
fn an_https_to_http_redirect_is_refused() {
    let origin = Url::parse("https://example.com/index.json").expect("origin");
    let error =
        http::redirect_target(&origin, "http://example.com/other.json").expect_err("refused");
    assert!(matches!(error, HttpError::Scheme(_)));
}

#[test]
fn a_loopback_redirect_is_followed() {
    let redirect = b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n".to_vec();
    let (url, handle) = respond(vec![
        redirect,
        response("200 OK", "Content-Type: application/json\r\n", b"final"),
    ]);
    let body = http::get(&format!("{url}/start"), MAX_DOCUMENT_BYTES).expect("get");
    handle.join().expect("server");
    assert_eq!(body, b"final");
}

#[test]
fn a_body_over_the_cap_is_refused() {
    let (url, handle) = respond(vec![response("200 OK", "", b"0123456789")]);
    let error = http::get(&format!("{url}/big"), 4).expect_err("too large");
    handle.join().expect("server");
    assert!(matches!(error, HttpError::TooLarge(_)));
}

#[test]
fn a_chunked_body_is_read() {
    let body = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n4\r\nefgh\r\n0\r\n\r\n".to_vec();
    let (url, handle) = respond(vec![body]);
    let value = http::get(&format!("{url}/chunked"), MAX_DOCUMENT_BYTES).expect("get");
    handle.join().expect("server");
    assert_eq!(value, b"abcdefgh");
}

#[test]
fn a_missing_resource_reports_the_status() {
    let (url, handle) = respond(vec![response("404 Not Found", "", b"")]);
    let error = http::get(&format!("{url}/missing"), MAX_DOCUMENT_BYTES).expect_err("missing");
    handle.join().expect("server");
    assert!(matches!(error, HttpError::Status(404)));
}

#[test]
fn a_download_streams_to_a_file() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let (url, handle) = respond(vec![response("200 OK", "", b"0123456789")]);
    let path = directory.path().join("asset.tar");
    let report =
        http::download(&format!("{url}/asset"), http::MAX_ASSET_BYTES, &path).expect("download");
    handle.join().expect("server");
    assert_eq!(report.bytes, 10);
    assert_eq!(std::fs::read(&path).expect("read"), b"0123456789");
    assert_eq!(report.digest, dosker_image::sha256_digest(b"0123456789"));
}

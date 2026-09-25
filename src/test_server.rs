//! A tiny local HTTP server for tests: no network, no fixtures on disk.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Reads one full request (head + Content-Length body) from the socket.
fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(pos) = find(&data, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..pos]).to_lowercase();
            let content_length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if data.len() >= pos + 4 + content_length {
                break;
            }
        }
    }
    data
}

/// Answers `times` requests by echoing each raw request back as JSON:
/// `{"request": "<method, path, headers and body as received>"}`. Returns
/// the base URL, e.g. `http://127.0.0.1:12345`.
pub fn serve_echo(times: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for _ in 0..times {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let raw = String::from_utf8_lossy(&read_request(&mut stream)).into_owned();
            let body = serde_json::json!({ "request": raw }).to_string();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: session=abc123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
}

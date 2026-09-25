//! Sends a built request over the network and turns the reply into a `ResponseData`.

use crate::json_view::pretty_json_if_possible;
use crate::model::{FieldKind, FormField, ResponseData, SendResult};
use crate::request::{OutgoingBody, OutgoingRequest};
use reqwest::blocking::multipart::Form;
use std::io::Read;
use std::sync::mpsc::Sender;
use std::time::Instant;

/// Bodies larger than this are cut off: they'd otherwise be held in memory
/// and laid out by the UI in full, which freezes the window.
pub const MAX_BODY_BYTES: u64 = 10 * 1024 * 1024;
const MAX_REDIRECTS: usize = 10;

pub fn send_request(req: OutgoingRequest, tx: Sender<SendResult>) {
    std::thread::spawn(move || {
        let _ = tx.send(execute(req));
    });
}

/// reqwest's own message is just "error sending request for url (...)"; the
/// useful part (refused, DNS failure, bad certificate, timeout) is in the
/// source chain, so join it all together.
fn describe_error(err: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(e) = source {
        let msg = e.to_string();
        if parts.last() != Some(&msg) {
            parts.push(msg);
        }
        source = e.source();
    }
    parts.join(": ")
}

fn multipart_form(fields: Vec<FormField>) -> Result<Form, String> {
    let mut form = Form::new();
    for f in fields {
        form = match f.kind {
            FieldKind::Text => form.text(f.key, f.value),
            FieldKind::File => {
                if f.value.trim().is_empty() {
                    return Err(format!("Field \"{}\" is a file field but no file is chosen.", f.key));
                }
                form.file(f.key, &f.value)
                    .map_err(|e| format!("Could not read file \"{}\": {e}", f.value))?
            }
        };
    }
    Ok(form)
}

fn execute(req: OutgoingRequest) -> SendResult {
    let redirect = if req.follow_redirects {
        reqwest::redirect::Policy::limited(MAX_REDIRECTS)
    } else {
        reqwest::redirect::Policy::none()
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(req.timeout)
        .redirect(redirect)
        .danger_accept_invalid_certs(req.insecure_tls)
        .build()
        .map_err(|e| e.to_string())?;

    let method = reqwest::Method::from_bytes(req.method.as_bytes()).map_err(|_| "Invalid HTTP method".to_string())?;

    let mut builder = client.request(method, &req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    builder = match req.body {
        OutgoingBody::None => builder,
        OutgoingBody::Text(text) => builder.body(text),
        OutgoingBody::Multipart(fields) => builder.multipart(multipart_form(fields)?),
    };

    let start = Instant::now();
    let res = builder.send().map_err(|e| describe_error(&e))?;
    let elapsed_ms = start.elapsed().as_millis();

    let status = res.status().as_u16();
    let status_text = res.status().canonical_reason().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = res
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
        .collect();
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Read raw bytes (capped) so the reported size is the real payload size
    // even when the body isn't valid UTF-8; decode lossily only for display.
    let total_size = res.content_length();
    let mut bytes = Vec::new();
    res.take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let truncated = bytes.len() as u64 > MAX_BODY_BYTES;
    if truncated {
        bytes.truncate(MAX_BODY_BYTES as usize);
    }
    let size_bytes = bytes.len();
    let text = String::from_utf8_lossy(&bytes).into_owned();

    let trimmed = text.trim_start();
    let looks_json = content_type.contains("json") || trimmed.starts_with('{') || trimmed.starts_with('[');
    let (body, json_value) = if looks_json {
        pretty_json_if_possible(&text)
    } else {
        (text, None)
    };

    Ok(ResponseData {
        status,
        status_text,
        elapsed_ms,
        size_bytes,
        headers,
        body,
        json_value,
        truncated,
        total_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BodyMode, PersistedState};
    use crate::request::build_request;
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    fn state() -> PersistedState {
        PersistedState {
            method: "POST".into(),
            url: "api.example.com/x".into(),
            ..Default::default()
        }
    }

    fn build(s: &PersistedState, bearer: &str) -> OutgoingRequest {
        build_request(s, bearer).unwrap()
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// Serves one canned response and reports the raw request it received.
    fn serve_once(head: String, body: Vec<u8>, stall: Option<Duration>) -> (String, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
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
                        .and_then(|v| v.trim().parse::<usize>().ok());
                    let done = match content_length {
                        Some(cl) => data.len() >= pos + 4 + cl,
                        None if head.contains("transfer-encoding: chunked") => data.ends_with(b"0\r\n\r\n"),
                        None => true,
                    };
                    if done {
                        break;
                    }
                }
            }
            let _ = tx.send(data);
            if let Some(d) = stall {
                std::thread::sleep(d);
            }
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        });
        (format!("http://127.0.0.1:{port}/"), rx)
    }

    fn ok_head(len: usize) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n")
    }

    fn req(url: String) -> OutgoingRequest {
        let mut r = build(&state(), "");
        r.method = "GET".into();
        r.url = url;
        r
    }


    #[test]
    fn execute_parses_a_json_response() {
        let (url, _) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 7\r\nConnection: close\r\n\r\n".into(),
            br#"{"a":1}"#.to_vec(),
            None,
        );
        let r = execute(req(url)).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.size_bytes, 7);
        assert!(r.json_value.is_some() && !r.truncated);
    }

    #[test]
    fn execute_truncates_oversized_bodies() {
        let total = MAX_BODY_BYTES as usize + 100;
        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n");
        let (url, _) = serve_once(head, vec![b'a'; total], None);
        let r = execute(req(url)).unwrap();
        assert!(r.truncated);
        assert_eq!(r.size_bytes as u64, MAX_BODY_BYTES);
        assert_eq!(r.total_size, Some(total as u64));
    }

    #[test]
    fn execute_can_stop_at_a_redirect() {
        let (url, _) = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/never\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            vec![],
            None,
        );
        let mut r = req(url);
        r.follow_redirects = false;
        assert_eq!(execute(r).unwrap().status, 302);
    }

    #[test]
    fn execute_times_out() {
        let (url, _) = serve_once(ok_head(0), vec![], Some(Duration::from_secs(4)));
        let mut r = req(url);
        r.timeout = Duration::from_secs(1);
        let started = Instant::now();
        assert!(execute(r).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn execute_reports_the_underlying_cause_not_just_a_generic_error() {
        let err = execute(req("http://127.0.0.1:1/".into())).err().unwrap();
        // reqwest alone says only "error sending request for url (...)".
        assert!(err.contains("error sending request") && err.contains("(Connect)"), "{err}");
    }

    #[test]
    fn multipart_upload_sends_text_and_file_parts_with_a_boundary() {
        let path = std::env::temp_dir().join(format!("mat-upload-{}.txt", std::process::id()));
        std::fs::write(&path, "FILEDATA-123").unwrap();

        let mut s = state();
        s.body_mode = BodyMode::Multipart;
        s.headers_text = "Content-Type: application/json".into();
        s.multipart_fields = vec![
            FormField { key: "title".into(), kind: FieldKind::Text, value: "hello".into(), enabled: true },
            FormField { key: "doc".into(), kind: FieldKind::File, value: path.to_string_lossy().into_owned(), enabled: true },
        ];
        let (url, rx) = serve_once(ok_head(2), b"ok".to_vec(), None);
        let mut r = build(&s, "");
        r.url = url;
        assert_eq!(execute(r).unwrap().status, 200);

        let raw = String::from_utf8_lossy(&rx.recv().unwrap()).into_owned();
        let lower = raw.to_lowercase();
        assert!(lower.contains("content-type: multipart/form-data; boundary="), "{raw}");
        assert_eq!(lower.matches("content-type: multipart/form-data").count(), 1);
        assert!(!lower.contains("application/json"), "{raw}");
        assert!(raw.contains("name=\"title\"") && raw.contains("hello"));
        let file_name = path.file_name().unwrap().to_string_lossy();
        assert!(raw.contains(&format!("name=\"doc\"; filename=\"{file_name}\"")), "{raw}");
        assert!(raw.contains("FILEDATA-123"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn multipart_reports_a_missing_or_unchosen_file() {
        let mut s = state();
        s.body_mode = BodyMode::Multipart;
        s.multipart_fields = vec![FormField {
            key: "doc".into(),
            kind: FieldKind::File,
            value: "Z:/definitely/not/here.bin".into(),
            enabled: true,
        }];
        let err = execute(build(&s, "")).err().unwrap();
        assert!(err.contains("Could not read file"), "{err}");

        s.multipart_fields[0].value = String::new();
        let err = execute(build(&s, "")).err().unwrap();
        assert!(err.contains("no file is chosen"), "{err}");
    }

}

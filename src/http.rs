use crate::json_view::pretty_json_if_possible;
use crate::model::ResponseData;
use std::sync::mpsc::Sender;
use std::time::Instant;

pub fn parse_headers(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (key, value) = line.split_once(':')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.trim().to_string()))
        })
        .collect()
}

pub fn format_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

pub fn send_request(
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    tx: Sender<Result<ResponseData, String>>,
) {
    std::thread::spawn(move || {
        let result = (|| -> Result<ResponseData, String> {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .map_err(|e| e.to_string())?;

            let method = reqwest::Method::from_bytes(method.as_bytes())
                .map_err(|_| "Invalid HTTP method".to_string())?;

            let mut builder = client.request(method, &url);
            for (k, v) in &headers {
                builder = builder.header(k, v);
            }
            if let Some(b) = body {
                builder = builder.body(b);
            }

            let start = Instant::now();
            let res = builder.send().map_err(|e| e.to_string())?;
            let elapsed_ms = start.elapsed().as_millis();

            let status = res.status().as_u16();
            let status_text = res.status().canonical_reason().unwrap_or("").to_string();
            let resp_headers: Vec<(String, String)> = res
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

            let text = res.text().map_err(|e| e.to_string())?;
            let size_bytes = text.len();

            let looks_json = content_type.contains("json")
                || text.trim_start().starts_with('{')
                || text.trim_start().starts_with('[');
            let (display_body, json_value) = if looks_json {
                pretty_json_if_possible(&text)
            } else {
                (text, None)
            };

            Ok(ResponseData {
                status,
                status_text,
                elapsed_ms,
                size_bytes,
                headers: resp_headers,
                body: display_body,
                json_value,
            })
        })();

        let _ = tx.send(result);
    });
}

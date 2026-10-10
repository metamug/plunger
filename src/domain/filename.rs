//! The name offered when a response body is saved to a file: the server's own file name when it
//! gives one, then the last part of the URL, then `response` with an extension that fits the
//! Content-Type. A JPEG from `/401.jpg` is saved as `401.jpg`, not `response.bin`.

/// A file name for the response to `url` with these headers; `is_json` and `is_binary` describe the body.
pub fn suggested(url: &str, headers: &[(String, String)], is_json: bool, is_binary: bool) -> String {
    let header = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str());

    if let Some(name) = header("content-disposition").and_then(from_disposition) {
        return name;
    }
    let content_type = header("content-type").map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
    let extension = content_type.as_deref().and_then(extension_for);
    if let Some(name) = from_url(url) {
        // A name with an extension is the best hint there is; one without borrows the type's.
        if name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty()) {
            return name;
        }
        if let Some(ext) = extension {
            return format!("{name}.{ext}");
        }
    }
    let ext = extension.unwrap_or(if is_json {
        "json"
    } else if is_binary {
        "bin"
    } else {
        "txt"
    });
    format!("response.{ext}")
}

/// `filename` from a Content-Disposition value, made safe to use as a file name.
fn from_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    for part in value.split(';').skip(1) {
        let (key, val) = part.split_once('=')?;
        let key = key.trim().to_ascii_lowercase();
        let val = val.trim();
        if key == "filename*" {
            // RFC 5987: charset'language'percent-encoded
            let encoded = val.splitn(3, '\'').nth(2).unwrap_or(val);
            if let Some(name) = percent_decode(encoded).and_then(|n| sanitize(&n)) {
                return Some(name);
            }
        } else if key == "filename" {
            plain = sanitize(val.trim_matches('"'));
        }
    }
    plain
}

/// The last path segment of the URL, if it looks like a name (not `{{variable}}`, not empty).
fn from_url(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = after_scheme.split(['?', '#']).next()?;
    let (_, path) = path.split_once('/')?;
    let segment = path.rsplit('/').next()?;
    if segment.contains("{{") {
        return None;
    }
    percent_decode(segment).and_then(|s| sanitize(&s))
}

fn sanitize(name: &str) -> Option<String> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let cleaned: String = name.chars().filter(|c| !c.is_control() && !"<>:\"|?*".contains(*c)).collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    (!cleaned.is_empty() && cleaned.len() <= 120).then_some(cleaned)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The usual extension for a media type.
fn extension_for(content_type: &str) -> Option<&'static str> {
    Some(match content_type {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/avif" => "avif",
        "image/bmp" => "bmp",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "image/tiff" => "tiff",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/x-tar" => "tar",
        "application/x-7z-compressed" => "7z",
        "application/json" | "application/problem+json" | "application/ld+json" => "json",
        "application/xml" | "text/xml" => "xml",
        "application/javascript" | "text/javascript" => "js",
        "application/wasm" => "wasm",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "text/html" => "html",
        "text/css" => "css",
        "text/csv" => "csv",
        "text/plain" => "txt",
        "text/markdown" => "md",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/ogg" => "ogg",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "font/woff2" => "woff2",
        "font/woff" => "woff",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_picture_keeps_the_name_from_its_url() {
        assert_eq!(suggested("https://http.dog/401.jpg", &h(&[("Content-Type", "image/jpeg")]), false, true), "401.jpg");
    }

    #[test]
    fn without_a_usable_name_the_content_type_picks_the_extension() {
        let image = h(&[("content-type", "image/png")]);
        assert_eq!(suggested("https://x.dev/avatar", &image, false, true), "avatar.png");
        assert_eq!(suggested("https://x.dev/", &image, false, true), "response.png");
        assert_eq!(suggested("https://x.dev/api/{{id}}", &image, false, true), "response.png");
        assert_eq!(suggested("https://x.dev/d", &h(&[("Content-Type", "application/pdf; charset=binary")]), false, true), "d.pdf");
    }

    #[test]
    fn the_servers_own_file_name_wins() {
        let hdr = h(&[("Content-Disposition", "attachment; filename=\"report 2026.pdf\""), ("Content-Type", "application/pdf")]);
        assert_eq!(suggested("https://x.dev/download?id=1", &hdr, false, true), "report 2026.pdf");
        let star = h(&[("Content-Disposition", "attachment; filename=\"fallback.txt\"; filename*=UTF-8''na%C3%AFve.csv")]);
        assert_eq!(suggested("https://x.dev/", &star, false, false), "naïve.csv");
    }

    #[test]
    fn a_file_name_cannot_escape_the_folder() {
        let hdr = h(&[("content-disposition", "attachment; filename=\"..\\..\\evil.exe\"")]);
        assert_eq!(suggested("https://x.dev/", &hdr, false, true), "evil.exe");
        let hdr = h(&[("content-disposition", "attachment; filename=\"../../etc/passwd\"")]);
        assert_eq!(suggested("https://x.dev/", &hdr, false, true), "passwd");
    }

    #[test]
    fn unknown_types_fall_back_as_before() {
        assert_eq!(suggested("https://x.dev/", &[], true, false), "response.json");
        assert_eq!(suggested("https://x.dev/", &[], false, false), "response.txt");
        assert_eq!(suggested("https://x.dev/", &h(&[("content-type", "application/x-weird")]), false, true), "response.bin");
        assert_eq!(suggested("https://x.dev/v1/users/", &[], true, false), "response.json", "a trailing slash has no name");
    }
}

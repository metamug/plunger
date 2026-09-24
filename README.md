# Metamug API Tester (desktop)

A tiny, fast, native desktop companion to [metamug.com/util/api-tester/](https://metamug.com/util/api-tester/).

Built with [egui](https://github.com/emilk/egui) — no webview, no bundled browser, just native rendering — because it's meant to be a quick, throwaway tool: open it, fire a request, close it. The reason it exists at all: a browser page (even with a CORS workaround) can never reliably reach `localhost` or an intranet API, and that's exactly where most APIs live while they're being developed. This app is a real native process, so it has no CORS, no mixed-content, and no Private Network Access restriction — it reaches whatever the machine it's running on can reach, the same way `curl` or Postman's desktop app do.

## What it is not

Not a Postman replacement. No collections, no environments, no scripting. If you need those, use Postman. This is for the thirty-second "let me just check this one endpoint" moment.

What it does have, because it earns its place in that thirty seconds: a request history sidebar (local SQLite), curl and HAR import, and `localhost:3000/api`-style URLs (a missing scheme is filled in: `http://` for local/private hosts, `https://` otherwise).

### Options tab

Per-session settings, kept when you load a request from history: timeout (1-600 s, default 20), follow redirects, and **Skip TLS certificate verification** for local APIs behind a self-signed certificate. When that is on, the tab is labelled "Options (TLS check off)" so it can't be forgotten.

### What is and isn't written to disk

- The Bearer field is never saved.
- Credential-looking headers (`Authorization`, `Cookie`, `X-Api-Key`, anything containing `token`/`secret`/`password`) are blanked before being written to history or the saved form state, and credential-looking query parameters and `user:password@` URL parts are blanked in history. A blanked credential header is not sent.
- Request bodies are stored as typed. Don't paste secrets into a body you don't want in the local history; **Clear** deletes it. History keeps the newest 1000 requests.
- Responses over 10 MB are cut off, with a notice showing the real size; large JSON opens collapsed one level deep instead of fully expanded.
- HTTPS uses the operating system's certificate store, so corporate CAs work.
- A crash writes `crash.log` next to the history database (`%APPDATA%\Metamug API Tester\data\` on Windows).

### Known limits

- **Cancel** stops the app waiting; the underlying request finishes or times out in the background (`reqwest::blocking` can't be interrupted).
- No multipart/file upload, cookie jar, or query-parameter editor.
- HTTP/1.1 only.
- Windows is the only platform tested.

## Development

```bash
cargo test      # unit tests for parsing, request building, history, JSON highlighting
cargo clippy --all-targets
```

Layout: `src/app/` is the UI (one file per panel), `src/http.rs` builds and sends requests, `src/history.rs` is the SQLite store, `src/curl_import.rs` parses curl/HAR.

## Building

Requires the Rust GNU toolchain on Windows (avoids needing the full Visual Studio Build Tools — just `rustup` + a mingw-w64 install):

```bash
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu
# plus a mingw-w64 gcc/ld on PATH (e.g. WinLibs, MSYS2)

cargo build --release
```

The binary is at `target/x86_64-pc-windows-gnu/release/metamug-api-tester.exe` (or `target/release/...` if built on the default host toolchain instead).

## Releasing

1. `cargo build --release`
2. Zip the `.exe` as `metamug-api-tester-windows.zip`
3. Upload to `s3://metamug-static-site/downloads/metamug-api-tester-windows.zip`
4. Invalidate CloudFront for `/downloads/metamug-api-tester-windows.zip`

Size: the exe is about 5.5 MB and the zip about 2.9 MB.

It's unsigned, so Windows SmartScreen will show an "Unknown publisher" warning on first run — expected until/unless a code-signing certificate is added. Not blocking, just a known first-run speed bump.

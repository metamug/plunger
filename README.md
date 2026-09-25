# Metamug API Tester (desktop)

A tiny, fast, native desktop companion to [metamug.com/util/api-tester/](https://metamug.com/util/api-tester/).

Built with [egui](https://github.com/emilk/egui) — no webview, no bundled browser, just native rendering — because it's meant to be a quick, throwaway tool: open it, fire a request, close it. The reason it exists at all: a browser page (even with a CORS workaround) can never reliably reach `localhost` or an intranet API, and that's exactly where most APIs live while they're being developed. This app is a real native process, so it has no CORS, no mixed-content, and no Private Network Access restriction — it reaches whatever the machine it's running on can reach, the same way `curl` or Postman's desktop app do.

## What it is not

Not a Postman replacement. No collections, no environments, no scripting. If you need those, use Postman. This is for the thirty-second "let me just check this one endpoint" moment.

What it does have, because it earns its place in that thirty seconds: a request history sidebar (local SQLite), curl and HAR import, and `localhost:3000/api`-style URLs (a missing scheme is filled in: `http://` for local/private hosts, `https://` otherwise).

### Params, form-data and variables

- **Params tab** - query parameters as rows (with an on/off checkbox each); they are URL-encoded and appended to the URL when you send.
- **form-data body** - a real `multipart/form-data` upload: text fields and file fields (pick a file, or import `curl -F name=@file`). The boundary and `Content-Type` are generated for you.
- **JSON body** - syntax highlighting, live validity check, and a Prettify button.
- **Variables** - define `name = value` in the Variables tab and use `{{name}}` in the URL, params, headers, body, form fields or Bearer token. Built-ins: `{{$uuid}}`, `{{$timestamp}}`, `{{$randomInt}}`. Sending is refused, with the names listed, if a variable is undefined, so a literal `{{token}}` is never transmitted. Variables ticked "secret", or named like `token`/`secret`/`password`/`key`, are never written to a file; tick **remember** to keep one in the system credential store, otherwise it is blank after a restart.
- History stores the `{{template}}`, not the resolved values, and loading a history entry keeps your current variables and options.

### Options tab

Per-session settings, kept when you load a request from history: timeout (1-600 s, default 20), follow redirects, and **Skip TLS certificate verification** for local APIs behind a self-signed certificate. When that is on, the tab is labelled "Options (TLS check off)" so it can't be forgotten.

### What is and isn't written to disk

- The Bearer field is not saved unless you tick **remember** next to it. Ticked values (and ticked secret variables) go into the operating system credential store (Windows Credential Manager / macOS Keychain), never into a file. **Forget saved secrets** on the Variables tab removes them all. Limits: about 2,500 bytes per secret, and remembering is not available on Linux yet.
- Credential-looking headers (`Authorization`, `Cookie`, `X-Api-Key`, anything containing `token`/`secret`/`password`), credential-looking params and form fields, and secret variable values are blanked before being written to history or the saved form state; `user:password@` URL parts are blanked in history. A blanked credential header is not sent. Put secrets in a **secret variable** (see Variables above) and they never touch the disk.
- Request bodies are stored as typed. Don't paste secrets into a body you don't want in the local history; **Clear** deletes it. History keeps the newest 1000 requests.
- Responses over 10 MB are cut off, with a notice showing the real size; large JSON opens collapsed one level deep instead of fully expanded.
- HTTPS uses the operating system's certificate store, so corporate CAs work.
- A crash writes `crash.log` next to the history database (`%APPDATA%\Metamug API Tester\data\` on Windows).

### Known limits

- **Cancel** stops the app waiting; the underlying request finishes or times out in the background (`reqwest::blocking` can't be interrupted).
- No cookie jar. Variables are one global set (no per-environment sets yet). `-F` is imported from curl; multipart from HAR files is not.
- HTTP/1.1 only.
- Windows is the only platform tested.

## Development

```bash
cargo test      # unit tests for parsing, request building, history, JSON highlighting
cargo clippy --all-targets
```

Layout: `src/app/` is the UI (`app/request/` has one file per request tab), `src/request.rs` turns the form into a request (variables, params, body; pure logic), `src/http.rs` sends it, `src/vars.rs` substitutes `{{variables}}`, `src/redact.rs` keeps credentials off disk, `src/history.rs` is the SQLite store, `src/curl_import.rs` parses curl/HAR.

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

Size: the exe is about 5.7 MB and the zip about 3.0 MB.

It's unsigned, so Windows SmartScreen will show an "Unknown publisher" warning on first run — expected until/unless a code-signing certificate is added. Not blocking, just a known first-run speed bump.

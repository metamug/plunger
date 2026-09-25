<h1 align="center">Plunger</h1>

<p align="center">
  <strong>Check one API endpoint in thirty seconds.</strong><br>
  A small native app for sending HTTP requests. No account, no cloud, no CORS limits.
</p>

<p align="center">
  <a href="https://github.com/metamug/plunger/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/metamug/plunger"></a>
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <img alt="Windows 10 and 11" src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-lightgrey">
  <img alt="Built with Rust and egui" src="https://img.shields.io/badge/built%20with-Rust%20%2B%20egui-orange">
</p>

<p align="center">
  <a href="https://github.com/metamug/plunger/releases/latest/download/plunger-windows.zip"><strong>Download for Windows</strong></a> (zip, about 3 MB, no installer)
  &nbsp;·&nbsp; <a href="https://metamug.com/util/api-tester-desktop/">Website</a>
  &nbsp;·&nbsp; <a href="store/privacy-policy.md">Privacy</a>
</p>

<!-- TODO: add a short GIF here: paste a curl command, press Send, the JSON tree appears. -->

---

Most of the time you don't need an API platform. You need to hit one endpoint, see the status code and read the JSON. Plunger is built for exactly that: open it, paste a URL or a curl command, press Send, close it.

It's a real native program, not a web page, so it goes wherever your machine can go: `localhost`, a dev server on your LAN, an intranet API, a service that sends no CORS headers. Like `curl`, with a window.

## Why Plunger

- **Starts clean, stays out of the way.** No sign-in, no workspace, no sync. One window, one request.
- **Reaches what a browser can't.** No CORS, mixed-content or private-network restrictions. `localhost:3000/api` just works (a missing scheme is filled in for you).
- **Your secrets stay yours.** Tokens are never written to a file unless you ask, and then only to Windows Credential Manager.
- **Small.** A single exe of about 6 MB, drawn natively with egui. No webview, no bundled browser.
- **Open source.** MIT licensed. Read exactly what it does with your requests.

## Features

| | |
|---|---|
| **Requests** | GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS. Header name suggestions as you type, and a Bearer token field. |
| **Bodies** | JSON (highlighting, live validation, Prettify), form-urlencoded, raw text, and real `multipart/form-data` with file uploads. |
| **Params** | Query parameters as a table, URL-encoded for you, each row switchable on or off. |
| **Variables** | `{{name}}` in the URL, params, headers, body, form fields or Bearer token. Built-ins `{{$uuid}}`, `{{$timestamp}}`, `{{$randomInt}}`. A request with an undefined variable is refused, so a literal `{{token}}` is never sent. |
| **Responses** | Collapsible JSON tree with status, time and size. Save the body to a file. |
| **Import** | Paste a curl command (including `-F` uploads) or open a HAR file. |
| **History** | The last 1,000 requests, stored locally. |
| **Local HTTPS** | Skip certificate verification for self-signed dev servers, with a tab label that reminds you it's off. Timeout and redirect controls. |
| **Certificates** | Uses the Windows certificate store, so corporate CAs work. |

## Private by design

- **No account, no analytics, no telemetry, no update checks.** The only network traffic is the requests you send.
- **Secrets never touch the disk by accident.** The Bearer token and secret variables live in memory. Tick **remember** to keep one in Windows Credential Manager; **Forget saved secrets** removes them all.
- **History is scrubbed before it's saved.** Headers, params and form fields that look like credentials (`Authorization`, `Cookie`, `X-Api-Key`, anything with `token`, `secret` or `password`) are blanked, and so are `user:password@` parts of URLs. History stores your `{{template}}`, not the resolved value.
- **One folder holds everything:** `%APPDATA%\Plunger\data` (history, last request, a crash log if it ever crashes). Delete it, plus **Forget saved secrets** if you used remember, and nothing is left.

Request bodies are saved to history as typed, so put secrets in a secret variable rather than pasting them into a body. Full details in the [privacy policy](store/privacy-policy.md).

## What Plunger is not

It is not a Postman replacement, on purpose. There are no collections, environments, scripting or team features. If you need those, [Postman](https://www.postman.com/) and [Bruno](https://www.usebruno.com/) are good at them. Plunger is for the moment before you'd reach for one of those.

## Install

1. [Download `plunger-windows.zip`](https://github.com/metamug/plunger/releases/latest/download/plunger-windows.zip) and unzip it anywhere.
2. Run `plunger.exe`.

Requires Windows 10 (1809) or 11, 64-bit.

**"Windows protected your PC"?** Plunger isn't code-signed yet, so SmartScreen may warn on first run. Click **More info**, then **Run anyway**. The source and the release checksums are here if you'd rather verify first, or [build it yourself](#build-from-source).

**Update:** download the new zip and replace the exe. **Uninstall:** delete the exe and `%APPDATA%\Plunger`, and use **Forget saved secrets** first if you ticked remember.

## Known limits

- **Cancel** stops waiting, but the request itself finishes or times out in the background.
- No cookie jar. Variables are one global set (no per-environment sets yet).
- Multipart bodies are imported from curl `-F`, not from HAR files.
- HTTP/1.1 only. Responses over 10 MB are truncated, with a notice showing the real size.
- Windows is the only platform built and tested so far.

## Build from source

Plunger builds with the Rust GNU toolchain on Windows, so you don't need Visual Studio Build Tools, only `rustup` and mingw-w64:

```bash
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu
# plus mingw-w64 (gcc, ld, windres, dlltool) on PATH, e.g. from WinLibs or MSYS2

cargo build --release
```

The exe is at `target/release/plunger.exe` (or `target/x86_64-pc-windows-gnu/release/` when cross-targeting).

```bash
cargo test                  # parsing, request building, history, redaction, variables
cargo clippy --all-targets
```

**Where things live:** `src/app/` is the UI (`app/request/` has one file per request tab), `src/request.rs` turns the form into a request, `src/http.rs` sends it, `src/vars.rs` substitutes variables, `src/redact.rs` keeps credentials off disk, `src/history.rs` is the SQLite store, and `src/curl_import.rs` parses curl and HAR.

## Contributing

Bug reports, fixes and polish are very welcome: [open an issue](https://github.com/metamug/plunger/issues). Please keep the scope in mind. Anything that makes the thirty-second check faster or safer fits; collections, scripting and sync don't. Run `cargo test` and `cargo clippy --all-targets` before sending a pull request.

## Releasing (maintainers)

1. Bump `version` in `Cargo.toml`, then `cargo build --release`.
2. Zip `plunger.exe` as `plunger-windows.zip` and note its SHA-256.
3. Create a GitHub release tagged `vX.Y.Z` with the zip attached and the checksum in the notes. Download links on the website use `releases/latest`, so they update automatically.

Microsoft Store packaging (MSIX) and the Store listing are in `packaging/` and `store/`; start with [`packaging/README.md`](packaging/README.md).

## License

[MIT](LICENSE). Made by [Metamug](https://metamug.com).

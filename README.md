# Metamug API Tester (desktop)

A tiny, fast, native desktop companion to [metamug.com/util/api-tester/](https://metamug.com/util/api-tester/).

Built with [egui](https://github.com/emilk/egui) — no webview, no bundled browser, just native rendering — because it's meant to be a quick, throwaway tool: open it, fire a request, close it. The reason it exists at all: a browser page (even with a CORS workaround) can never reliably reach `localhost` or an intranet API, and that's exactly where most APIs live while they're being developed. This app is a real native process, so it has no CORS, no mixed-content, and no Private Network Access restriction — it reaches whatever the machine it's running on can reach, the same way `curl` or Postman's desktop app do.

## What it is not

Not a Postman replacement. No collections, no environments, no scripting, no history. If you need those, use Postman. This is for the thirty-second "let me just check this one endpoint" moment.

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

It's unsigned, so Windows SmartScreen will show an "Unknown publisher" warning on first run — expected until/unless a code-signing certificate is added. Not blocking, just a known first-run speed bump.

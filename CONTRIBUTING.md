# Contributing to Plunger

Thanks for wanting to help. Plunger is a small, open-source, MIT-licensed tool, and it stays small on purpose. This page explains what fits, how to build it, and how to send a change.

## What belongs here

Plunger does one job: **quickly see what an API is doing.** Before you write code, ask:

- Does it help someone see what an API did, faster?
- Would someone open Plunger specifically for it?

If yes, it probably belongs. Things that don't: accounts, cloud sync, collaboration, team features, monitors, mock servers, and anything that adds telemetry. The project promises never to require an account, never to add telemetry, and never to move existing features behind a paywall. Changes that break those promises won't be merged.

**For anything bigger than a small fix, open an issue first** so we can agree it fits before you spend the time. Bug reports and small fixes can go straight to a pull request.

## Ways to help

- **Report a bug** with the [bug template](https://github.com/metamug/plunger/issues/new?template=bug_report.yml). The most useful reports say what you sent (with secrets removed), what you expected, and what happened.
- **Suggest an improvement** with the [feature template](https://github.com/metamug/plunger/issues/new?template=feature_request.yml).
- **Fix an issue.** Look for the `good first issue` label.
- **Try it on your setup** (Windows 10/11 is the supported platform) and tell us what's rough.
- **Improve the docs**, including [docs/agents.md](docs/agents.md).

## Building

Plunger is Rust. On Windows:

```bash
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu
# plus mingw-w64 (gcc, windres, dlltool) on PATH, e.g. from WinLibs or MSYS2

cargo build --release        # target/release/plunger.exe
cargo test
cargo clippy --all-targets -- -D warnings
```

Run `cargo test` and `cargo clippy` before you send a pull request; CI runs the same checks with warnings as errors.

To try your build without touching your own history and settings, point it at a scratch folder:

```bash
PLUNGER_DATA_DIR=/path/to/scratch cargo run
```

## Finding your way around

[design.md](design.md) explains the architecture and the reasons behind it. The short version:

- `src/app/` is the window (egui). It holds no request logic.
- `request.rs`, `vars.rs`, `http.rs`, `model.rs` and friends are the **core**: they know nothing about the window or agents.
- `engine.rs`, `agent.rs`, `cli.rs` and `mcp.rs` are the agent interface. They use the same core as the window, so a rule like "an undefined `{{variable}}` is never sent" holds in every mode. Keep it that way: a behaviour change to sending belongs in the core, not in one front end.

## Sending a pull request

1. Fork, and branch from `main`.
2. Keep the change focused: one fix or feature per pull request, without unrelated cleanup.
3. Add a test for new behaviour and for a fixed bug. HTTP behaviour can be tested against the in-process server in `src/test_server.rs`.
4. Make sure `cargo test` and `cargo clippy --all-targets -- -D warnings` pass.
5. If it changes what users see or do, update the README or `docs/`, and add a line under **Unreleased** in [CHANGELOG.md](CHANGELOG.md).
6. In the description, say what changed and why, and how you tested it. For UI changes, a screenshot helps.

By contributing, you agree that your contribution is licensed under the [MIT license](LICENSE), the same as the rest of the project.

### Guidelines that keep it healthy

- **Secrets are the sharp edge.** Anything that reads, stores, logs or returns a credential needs extra care and a test. Credentials must never reach disk, history or agent output unless the user opted in to storing them.
- **No new network calls** except the requests the user (or their agent) sends. No analytics, no update checks.
- **Keep it small.** A new dependency needs a good reason; the size of the exe is a feature.
- **Match the code around you.** Comments explain *why*, not what. Prefer a small function that's reused over a copy.

## Security problems

Please don't open a public issue for a vulnerability. See [SECURITY.md](SECURITY.md).

## Conduct

Be kind and assume good faith. See the [Code of Conduct](CODE_OF_CONDUCT.md).

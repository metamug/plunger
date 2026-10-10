# Plunger on macOS

## What you get

Each release has, for macOS:

| File | What it is |
|---|---|
| `Plunger-<version>-macos.dmg` | A disk image with `Plunger.app` (one app for Apple Silicon and Intel) and a shortcut to Applications. Drag the app across. |
| `plunger-macos-arm64.tar.gz`, `plunger-macos-x86_64.tar.gz` | The bare `plunger` program for one kind of Mac, for the command line, scripts and MCP servers. |
| `brew install --cask metamug/tap/plunger` | The same app through Homebrew (the tap is [metamug/homebrew-tap](https://github.com/metamug/homebrew-tap)), plus a `plunger` command on the PATH. |
| `pip install plunger-cli` | The same program as a Python package (`pipx` and `uvx` work too). |

macOS 11 (Big Sur) or later.

## Opening it the first time

The app is signed *ad hoc*: it carries a signature (Apple Silicon will not run code without one), but
not an Apple Developer ID, so Gatekeeper does not know who made it. The first time:

- Right-click `Plunger.app` and choose **Open**, then **Open** again; or
- **System Settings > Privacy & Security**, scroll to the message about Plunger, **Open Anyway**; or
- from a terminal: `xattr -dr com.apple.quarantine /Applications/Plunger.app`

After that it opens normally. Downloads made with `curl`, `pip` or Homebrew's `--no-quarantine` are not
marked, so they never show the prompt.

## Using the app's program from a terminal

The program inside the app is the same one as the command-line tool:

```bash
/Applications/Plunger.app/Contents/MacOS/plunger send --url https://example.com
```

For an MCP client, the command is that path with the argument `mcp` (or run **Tools > Set up AI
agents**, which fills in the path of the copy you are running).

Settings, history and saved requests are in `~/Library/Application Support/Plunger`.

## How it is built

`packaging/macos/build-app.sh` makes the app from the two compiled programs: it joins them with `lipo`
into one universal binary, writes the `Info.plist`, builds the `.icns` icon from
`packaging/icons/app-1024.png`, signs ad hoc with `codesign`, and makes the disk image with `hdiutil`.
The release workflow runs it on a macOS runner after both macOS builds, checks the result (universal,
signature valid, the window starts, the disk image mounts) and attaches it to the release.

To build one yourself on a Mac:

```bash
cargo build --release
packaging/macos/build-app.sh 0.5.3 out target/release/plunger
open out/Plunger.app
```

## What signing and notarization would add

With an Apple Developer ID (a paid membership), the app could be signed with it and *notarized*, and
then it opens with no prompt at all. It needs these repository secrets, then two more steps in the
`macos-app` job (import the certificate into a keychain, `codesign --options runtime --sign "Developer ID
Application: ..."`, then `xcrun notarytool submit ... --wait` and `xcrun stapler staple`):

- `MACOS_CERTIFICATE` (the .p12, base64) and `MACOS_CERTIFICATE_PASSWORD`
- `APPLE_ID`, `APPLE_TEAM_ID` and an app-specific password for `notarytool`

Until then the one-time approval above is the cost of a free, unsigned distribution.

# Packaging and Store submission

Everything needed to ship Plunger in the Microsoft Store (where Microsoft signs the package, so no code-signing certificate is needed) lives here and in `store/`.

## What is in the repo

| Path | Purpose |
|---|---|
| `icons/make-icons.ps1` | Generates `app.ico`, the 1024 px and 300 px logos, and every MSIX tile size. Pure PowerShell, no tools needed. |
| `icons/app.ico` | Embedded into the exe by `build.rs` (Explorer, taskbar, Task Manager) together with the version info. |
| `msix/AppxManifest.template.xml` | The MSIX manifest. Declares a full-trust desktop app. |
| `msix/build-msix.ps1` | Stages the release exe and assets, fills in your Partner Center identity, sanity-checks the manifest, and packs the `.msix`. |
| `msix/fetch-buildtools.ps1` | Downloads Microsoft's `makeappx` (NuGet package `Microsoft.Windows.SDK.BuildTools`) into `packaging/tools/` (git-ignored) if you do not have the Windows SDK. |
| `../store/listing.md` | Every field to paste into Partner Center. |
| `../store/privacy-policy.md` | Text for the privacy page on metamug.com (the Store requires a URL). |
| `../store/screenshots/SHOTLIST.md` | The screenshots to capture, with exact setup. |

## Steps

### 1. One time: Partner Center

1. Create a developer account at Partner Center (free) and complete identity verification.
2. Reserve the app name **Plunger** (or **Plunger API Tester** if "Plunger" is taken).
3. Open the app's **Product identity** page and note three values: *Package/Identity/Name*, *Package/Identity/Publisher* (starts with `CN=`) and *Package/Properties/PublisherDisplayName*.

### 2. Build the package

```powershell
cargo build --release          # needs mingw-w64 (windres) on PATH, see the main README
powershell -File packaging\msix\fetch-buildtools.ps1     # only if you have no Windows SDK
powershell -File packaging\msix\build-msix.ps1 `
    -IdentityName "<Package/Identity/Name>" `
    -Publisher "<Package/Identity/Publisher>" `
    -PublisherDisplayName "<PublisherDisplayName>"
```

The result is `target\msix\Plunger_<version>_x64.msix`. Do not sign it; the Store signs it.

`-Dev` builds with placeholder identity values for a local dry run. That package cannot be submitted.

### 3. Check before submitting (recommended)

Run the Windows App Certification Kit (part of the Windows SDK) on the `.msix`, and test the packaged app: history should save, file pickers and the credential store should work, and requests to localhost should succeed. Inside MSIX, Windows redirects writes under `%APPDATA%` into the package's private storage; the app should behave the same, but its history is stored separately from the zip version.

### 4. Submit

Create a submission in Partner Center: upload the `.msix`, then fill in the listing from `store/listing.md`, add the screenshots, set the privacy policy URL, answer the age-rating questionnaire, and paste the certification notes. Certification takes up to three business days.

### 5. Every release

Bump the version in `Cargo.toml` (the fourth MSIX number stays 0), rebuild, run `build-msix.ps1` again, and submit an update. The zip on metamug.com is uploaded separately (see the main README).

## What is not verified yet

- The package has not been packed or installed: this machine has no Windows SDK, and `makeappx` has not been downloaded. `build-msix.ps1` validates the manifest and every asset reference, but only `makeappx` can confirm the package is valid.
- Store certification has not been attempted, so approval is unproven.

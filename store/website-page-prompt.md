# Prompt for the metamug.com agent

Paste everything below the line into the agent that works on `D:\projects\metamug.com`.

---

You are working in the PHP website repo at `D:\projects\metamug.com`. Your job: add a landing/download page for the **Metamug API Tester desktop app** and link it from the existing browser tool at https://metamug.com/util/api-tester/. Do not fake anything: use only the facts in this brief. Do not commit or push (the repo has no commits on its branch and all files show as untracked, so never run `git add -A`). Do not run any AWS commands and do not add the zip to the repo; it is served from S3.

## 1. Read these first, and mimic them

- `util/api-tester/index.php`: the existing browser tool. It already has a warning box `#desktop-app-callout` that links straight to `/downloads/metamug-api-tester-windows.zip`, and an OS-detection script that fills `#desktop-app-cta`. Keep that direct download working.
- One simple sibling tool page such as `util/url-parser/index.php`, for the page skeleton: `<body class="tool-page">`, `include '../../fragments/leap/leap-header.php'`, `assets/css/custom.css`, `theme.css`, `tools.css`, Bootstrap **4.3.1** classes, and the Google Analytics / AdSense snippets in `<head>`. Copy those snippets exactly as siblings do; invent nothing.
- `util/index.php`: has a `['slug' => ..., 'name' => ..., 'desc' => ...]` array listing every tool.
- `fragments/page-meta.php` and `sitemap.xml`: check whether pages use them for meta tags and how URLs are listed.
- `legal/privacy-policy.php`: the website's own privacy policy (the site uses analytics and ads; the desktop app does not).

## 2. Create

1. `util/api-tester-desktop/index.php`: the landing page (spec below). Live URL: `https://metamug.com/util/api-tester-desktop/`.
2. `util/api-tester-desktop/privacy/index.php`: the app's privacy page. Render `D:\projects\metamug-api-tester\store\privacy-policy.md` as HTML, with the text kept verbatim. Live URL: `https://metamug.com/util/api-tester-desktop/privacy/`. Same page skeleton; include a link to the website policy.
3. Images in `assets/img/api-tester-desktop/`:
   - `icon-512.png` and a favicon-size version, from `D:\projects\metamug-api-tester\packaging\icons\app-1024.png`.
   - Screenshots from `D:\projects\metamug-api-tester\store\screenshots\` (files named `01-response.png` ... `07-options-tls.png`, described in `SHOTLIST.md` in that folder, including the caption for each). **If a screenshot file does not exist yet, do not mock one up.** Render an accessible placeholder figure with the caption and leave an HTML comment `<!-- TODO screenshot NN -->`, and list the missing ones in your report. Downscale large PNGs to at most 1600 px wide, add `width`/`height` attributes and `loading="lazy"`, and write meaningful `alt` text.

## 3. Edit (small, surgical)

- `util/api-tester/index.php`: in `#desktop-app-callout`, add a link "See everything the desktop app does" to `/util/api-tester-desktop/`. Do not touch the send/receive logic or the OS-detection script.
- `util/index.php`: add an entry `['slug' => 'api-tester-desktop', 'name' => 'API Tester for Windows', 'desc' => 'Free native desktop HTTP client. Reaches localhost, no CORS limits.']`, placed next to the api-tester entry.
- `sitemap.xml`: add the two new URLs in the file's existing format.

Touch no other files.

## 4. Landing page spec

**Tone:** plain, honest, developer-to-developer. No hype, no competitor numbers, no invented statistics.

**Title tag** (at most 60 characters): `Metamug API Tester for Windows | Free Desktop HTTP Client`
**Meta description** (at most 160): `A fast, native desktop tool for sending HTTP requests. No account, no cloud, no CORS limits. Reaches localhost and intranet APIs. Free for Windows.`
Add canonical, Open Graph and Twitter card tags (use screenshot 01 for the share image if it exists), and JSON-LD `SoftwareApplication` (`applicationCategory` DeveloperApplication, `operatingSystem` "Windows 10, Windows 11", `offers` price 0 USD, `downloadUrl` the zip, `fileSize` about 3 MB). Do not include ratings or reviews.

**Sections, in order**

1. **Hero:** H1 "Metamug API Tester for Windows". Sub-line: "The quick way to check one endpoint. A small native app: no account, no cloud, no CORS limits." Primary button **Download for Windows (zip, 3 MB)** linking to `/downloads/metamug-api-tester-windows.zip`. Below it: "Version 0.1.0 · Windows 10 (1809) or 11, 64-bit · No installer: unzip and run". Second button "Get it from the Microsoft Store" driven by a PHP variable `$storeUrl = '';` at the top of the file: when empty, show nothing or a muted "Microsoft Store: coming soon"; when set, show a proper link. Hero image: screenshot 01.
2. **Why a desktop app:** a browser page can never reach `localhost` or an intranet API, and is bound by CORS. This is a real native program, so it reaches whatever your machine can reach, the way curl does. Link back to the browser tool: "For public APIs that allow browser requests, the [online API tester](/util/api-tester/) needs no download."
3. **What it does** (feature blocks with screenshots, in this order: 01, 03, 02, 04, 05, 06, 07; use the captions from `SHOTLIST.md`):
   - Requests: GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS; headers with suggestions; a Bearer token field.
   - JSON response tree with status, time, size; save the body to a file.
   - Params table (query parameters, URL-encoded, individually switchable).
   - multipart/form-data with real file uploads; also JSON, form-urlencoded and raw bodies.
   - `{{variables}}` in the URL, params, headers, body and form fields; built-ins `{{$uuid}}`, `{{$timestamp}}`, `{{$randomInt}}`; a request with an undefined variable is refused instead of sent.
   - Import: paste a curl command (including `-F`) or open a HAR file.
   - Local history; self-signed HTTPS option for local dev servers; timeout and redirect controls.
4. **Private by design:** no account, no analytics, no telemetry; requests go only where you point them; Bearer tokens and secret variables are not written to files, and optionally kept in the Windows Credential Manager if you tick "remember"; credential-looking headers and parameters are blanked before history is saved. Link to `/util/api-tester-desktop/privacy/`. State plainly that the *website* uses analytics and ads as described in the site privacy policy, while the *app* does not.
5. **Not a Postman replacement:** no collections, environments, scripting or team features. If you need those, use Postman or Bruno. This is for the thirty-second "let me check one endpoint" moment. Name competitors neutrally, with no comparison table and no performance claims.
6. **Install and the SmartScreen warning:** unzip anywhere and run `metamug-api-tester.exe`. Because the app is new and not yet code-signed, Windows may show "Windows protected your PC"; click **More info**, then **Run anyway**. Say that the Microsoft Store version (when available) avoids this. Do not claim the app is signed.
7. **FAQ** (use `<details>` or plain headings; also emit as JSON-LD `FAQPage` only if the site already does that elsewhere): Is it free? Does it send my requests to Metamug? (No.) Where is my data stored? (`%APPDATA%\Metamug API Tester`; clear history with the Clear button.) How do I update? (Download the new zip and replace the exe.) Mac or Linux? (Not yet; Windows only for now.) How do I remove it? (Delete the exe and the data folder.)
8. **Changelog:** `0.1.0`, first release: requests, params, JSON/form/multipart bodies with file upload, variables, curl and HAR import, history, self-signed certificate option, remembered secrets.
9. **Footer/contact:** support@metamug.com.

## 5. Claims you must not make

Speed or memory numbers versus any competitor; that it is signed; macOS or Linux availability; that it is open source; ratings, reviews, download counts.

## 6. Quality bar and verification

- Responsive from 375 px to desktop, semantic headings (one H1), sufficient contrast, keyboard-usable buttons, images with alt text.
- Serve locally with `php -S localhost:8080` from the repo root (PHP 8.1 is installed). Open `/util/api-tester-desktop/`, `/util/api-tester-desktop/privacy/`, `/util/api-tester/` and `/util/` in the built-in browser at desktop width and at 375 px width. Check: no console errors, no broken images, every link resolves, the download link path is correct (the zip itself will 404 locally; that is expected), and the existing browser tool still sends a request.
- Take a screenshot of each new page at desktop and mobile width and include them in your report.

## 7. Report back

List: files created, files edited (with a one-line description of each edit), screenshots still missing, anything in this brief you could not do or that conflicted with the repo's conventions, and any question for me.

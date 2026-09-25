# Microsoft Store listing: Metamug API Tester

Copy these into Partner Center. Limits noted are the Store's; check them at submission time.

## Basics

| Field | Value |
|---|---|
| Product name | Metamug API Tester |
| Category | Developer tools |
| Price | Free |
| Support email | support@metamug.com |
| Website | https://metamug.com/util/api-tester-desktop/ |
| Privacy policy URL | https://metamug.com/util/api-tester-desktop/privacy/ |
| Minimum OS | Windows 10 version 1809 (build 17763) or later, x64 |
| Languages | English |

## Short description (about 200 characters)

A fast, native tool for sending HTTP requests. No account, no cloud, no CORS limits. Reaches localhost and intranet APIs. Small download.

## Description

Metamug API Tester is the quick way to check one endpoint. Open it, type a URL, press Send. There is no sign-in, no workspace to set up and no cloud: it is a small native Windows app that talks straight to your API from your own machine.

Because it is a real desktop program and not a web page, it has none of the browser's limits. It reaches localhost, private-network and intranet APIs, and it is not blocked by CORS.

WHAT YOU CAN DO
- Send GET, POST, PUT, PATCH, DELETE, HEAD and OPTIONS requests.
- Build query strings in a Params table; values are URL-encoded for you.
- Send JSON, form-urlencoded, raw text or multipart/form-data, including file uploads.
- Use {{variables}} anywhere: URL, params, headers, body and form fields. Built-ins such as {{$uuid}} and {{$timestamp}} are included. A request with an undefined variable is refused instead of sent with a literal {{token}}.
- Read responses with a collapsible JSON tree, status, timing and size, and save the body to a file.
- Import a curl command (including -F form uploads) or a HAR file.
- Keep a history of past requests, stored on your machine.
- Test local HTTPS servers that use a self-signed certificate with one checkbox.

PRIVATE BY DESIGN
- No account, no analytics, no telemetry.
- Requests go only where you point them.
- Tokens and other credentials are never written to files. If you choose "remember", they are kept in the Windows Credential Manager.

Metamug API Tester is a focused tool, not a Postman replacement: it has no collections, scripting or team features.

## Features list (up to 20 bullets, each up to 200 characters)

1. Native and lightweight: a few MB, starts instantly, no account or sign-in
2. Reaches localhost, private-network and intranet APIs; no browser CORS restrictions
3. GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS with headers, params and bodies
4. multipart/form-data with real file uploads, plus JSON, form-urlencoded and raw bodies
5. {{variables}} with built-ins like {{$uuid}}; undefined variables are caught before sending
6. Collapsible JSON response viewer with timing, size, headers and save-to-file
7. Import curl commands (including -F) and HAR files
8. Local request history; credentials are blanked before anything is saved
9. Optional remember for tokens using the Windows Credential Manager
10. Self-signed certificate toggle for local HTTPS development servers

## Keywords (up to 7)

api tester, http client, rest client, postman alternative, curl, json, developer tools

## What's new (version 0.1.0)

First release: requests, query params, JSON / form / multipart bodies with file upload, variables, curl and HAR import, history, self-signed certificate option, and optional remembered secrets.

## Notes for certification

- No sign-in or test account is needed. Launch the app and press Send: the default request calls a public demo API (https://jsonplaceholder.typicode.com/todos/1) and needs an internet connection. Without a connection the app shows a request error and stays usable.
- The package declares the restricted capability runFullTrust because this is a standard Win32 desktop application. It sends HTTP requests to whatever URL the user types (including localhost and private-network hosts), reads files the user chooses for multipart uploads, and stores its own history and settings in its local data folder.
- The app collects no personal data and has no telemetry. See the privacy policy URL.
- To test file upload: Body tab, choose "form-data", add a File field, choose any file, and send to https://httpbin.org/post.
- To test the credential store: Variables tab, add a variable named "token", tick "remember", type a value, close and reopen the app.

## Age rating (IARC questionnaire): answer truthfully; expected answers

This is a developer utility. It displays raw HTTP responses from URLs the user enters, so where the questionnaire asks about unrestricted internet access or user-generated content, answer yes and explain that the app is not a web browser and shows only the text of the response the user requested. Violence, sexual content, gambling, purchases, chat and location: no.

## Screenshots (at least 1; PNG, 1366x768 or larger; up to 10)

See `store/screenshots/SHOTLIST.md`. Each screenshot needs a short caption (about 100 characters).

## Store logos

- 1:1 logo 300x300: `packaging/icons/store-logo-300.png`
- Larger 1024x1024 source: `packaging/icons/app-1024.png`
- Tile assets are packaged from `packaging/msix/Assets/`.

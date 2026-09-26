# Privacy policy: Plunger (desktop app)

Last updated: 2026-09-25

This policy covers the Plunger desktop application for Windows. It does not cover the metamug.com website; see the [website privacy policy](https://metamug.com/legal/privacy-policy.php) for that.

## The short version

The app collects nothing. There is no account, no analytics and no telemetry, and Metamug never receives your requests, responses, files or credentials.

## What the app sends over the network

Only the HTTP requests that get sent from Plunger: the ones you create and send from the window, and, if you connect an AI agent through the command line or the MCP server, the ones it creates. Each request goes directly from your computer to the address in the request. Metamug is not in the middle and does not see it. What the receiving server does with your request is governed by that server's own policies. The app makes no other network connections: no update checks, no crash reporting, no usage statistics.

## What the app stores on your computer

All of the following stays on your device, in the app's local data folder:

- **Your open tabs and settings**, so the app reopens where you left off.
- **Request history** (up to 1,000 recent requests): method, URL, headers, parameters and body as you typed them, the status code and the response time, and whether it was sent from the window, the command line or MCP. Before saving, the app blanks the values of headers, parameters and form fields that look like credentials (such as Authorization, Cookie, API keys, tokens and passwords), and any password inside a URL.
- **Saved requests** you name with Save, with the same credential blanking as history. They stay until you delete them, even when you clear the history.
- **Request bodies are saved as you typed them.** If you paste a secret into a body, it will be in your local history until you clear it.
- **A local crash log**, written only if the app fails, containing the error message and the app version.

## Credentials

Bearer tokens and variables marked secret are not written to any file. They are held in memory and are gone when you close the app, unless you tick "remember" for that value. Remembered values are stored in the Windows Credential Manager, which is part of Windows and protected by your Windows account. "Forget saved secrets" in the app removes them. An AI agent can use only the secrets you have chosen to remember, by name; it never sees the value.

## Files

The app reads a file only when you choose one, for example to attach it to a multipart upload or to import a curl or HAR file, and it writes a file only when you use Save. It does not scan or index your files.

## Your control

- Clear the request history at any time with the Clear button.
- Remove remembered secrets with "Forget saved secrets".
- Plunger has no installer. To remove it, delete the exe and the app data folder (`%APPDATA%\Plunger`).

## Third parties and children

The app includes no third-party analytics, advertising or tracking. It is a developer tool and is not directed at children.

## Changes and contact

If this policy changes, the date above will change and the new text will be published at this address. Questions: privacy@metamug.com.

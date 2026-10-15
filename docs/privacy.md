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
- **Request history** (up to 1,000 recent requests): method, URL, headers, parameters and body, the status code and the response time, and whether it was sent from the window, the command line or MCP. Before saving, the app blanks the values of headers, parameters and form fields that look like credentials (such as Authorization, Cookie, API keys, tokens and passwords), and any password inside a URL. **Bodies are masked the same way**: in a JSON body, a form body or XML, the value of a member or field whose name looks like a credential (`password`, `token`, `secret`, `api_key`, `credential`, `passphrase`, `private_key`, `access_key`, `pwd`, `auth`, `authorization`, `cookie`, `otp`, `cvv`) is blanked before it is saved, in the history, in saved requests and in the audit log. Only string values are blanked (`"tokens_used": 5` is left alone), a `{{placeholder}}` is a reference and is kept, and the request that is *sent* is never changed. The audit log of an earlier version can still hold a value that was saved before this masking existed: the history database is scrubbed when it is opened, but a hash-chained log cannot be edited, so delete `audit.log` (a new one starts) if that matters to you.
- **Saved requests** you name with Save, with the same credential blanking as history. They stay until you delete them, even when you clear the history.
- **A secret under a name the masking does not recognise is saved.** Masking goes by the name of the member or field. A credential in a body under an ordinary name (`{"data": "s3cr3t"}`), or inside free text, is saved as typed. Put credentials in a `{{variable}}` and the saved request holds only the reference.
- **An audit log** (`audit.log`, beside the history database): one line for every change to the history, saved requests, workflows and the variables agents set, in order, each line chained to the one before it so an edited, removed or reordered line can be found (`plunger audit verify`). It holds what the history holds, with the same credential blanking: no response bodies and no secret values. **Clear history only hides the list; the log keeps the lines.** To remove it, delete `audit.log` (a new one starts), or delete the whole data folder. The database is derived from it (`plunger audit rebuild`). No agent tool can read or write it. When it passes 20 MB it is moved aside as `audit.<number>.log` and a new one begins; the old files are yours to keep or delete.
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

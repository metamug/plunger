# Screenshot shot list

Use for the Microsoft Store listing and the metamug.com page.

**Format:** PNG, at least 1366x768 (Store minimum for desktop), dark theme (the app's only theme), window fully visible, no other apps or personal data in frame. Save as `NN-name.png` in this folder. Captions are about 100 characters.

**Before capturing:** use a clean history (Clear button) so only the requests below appear, and set the window to 1366x768 or larger.

All requests use public demo APIs so anyone can reproduce them. The demo services are https://jsonplaceholder.typicode.com and https://httpbin.org.

| # | File | What to show | How to set it up | Caption |
|---|---|---|---|---|
| 1 | `01-response.png` | A finished GET with the collapsible JSON response, status badge, time and size | Method GET, URL `https://jsonplaceholder.typicode.com/todos/1`, Send. Expand nothing extra; the tree opens fully for small JSON. | Send a request and read the response as a collapsible JSON tree |
| 2 | `02-params.png` | The Params tab with two rows and one disabled row | URL `https://jsonplaceholder.typicode.com/comments`. Params: `postId` = `1`, `_limit` = `5`, and a third row `_sort` = `id` with its checkbox unticked. Send. | Query parameters as a table; values are URL-encoded for you |
| 3 | `03-form-data.png` | Body tab, form-data mode, a text field and a file field | Method POST, URL `https://httpbin.org/post`. Body > form-data: `title` = `Quarterly report` (Text), `document` (File, choose any small file such as a .pdf or .txt). Send so the echo shows in the response. | multipart/form-data with real file uploads |
| 4 | `04-variables.png` | The Variables tab with plain and secret variables | Variables: `base` = `https://jsonplaceholder.typicode.com`, `token` = `demo-token` (masked, "secret" locked on, "remember" ticked). URL field: `{{base}}/todos/1`. Tab label shows "Variables (2)". | {{variables}} anywhere; secrets stay out of files and can be remembered in Windows |
| 5 | `05-headers-bearer.png` | The Headers tab: the Bearer card with "remember", and header suggestions | Headers tab. Bearer field filled (shows dots), "remember" ticked. Add a header row and type `Acc` so suggestion chips appear. | Bearer token and header suggestions, with credentials kept out of history |
| 6 | `06-history-import.png` | The history sidebar with several rows plus the Import window | Send 4 to 5 of the requests above so the sidebar is populated, then click Import and paste `curl https://httpbin.org/post -F title=Hello -F file=@README.md`. | Local history, and paste any curl command or HAR file to import it |
| 7 | `07-options-tls.png` | The Options tab with the TLS checkbox on and its warning | Options tab: timeout 20 s, follow redirects ticked, "Skip TLS certificate verification" ticked. Tab label reads "Options (TLS check off)". | Test local HTTPS servers with self-signed certificates |

**Order on the page and in the Store:** 1, 3, 2, 4, 5, 6, 7. The first two carry the most weight.

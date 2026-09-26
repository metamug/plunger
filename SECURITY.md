# Security policy

Plunger handles credentials (bearer tokens, API keys, secret variables) and can be driven by AI agents, so security reports are taken seriously.

## Reporting a vulnerability

**Please don't open a public issue.** Use GitHub's private reporting instead: go to the repository's **Security** tab and choose **Report a vulnerability**. That opens a private conversation with the maintainers.

Include what you can:

- What you found and why it matters.
- Steps to reproduce, or a proof of concept.
- The Plunger version (`plunger --version`) and Windows version.

We aim to reply within a week. We'll work with you on a fix and a coordinated disclosure, and credit you in the release notes unless you'd rather stay anonymous.

## What counts

Reports we especially want to hear about:

- A **credential reaching disk, history or the window state file** without the user opting in to remember it.
- A **secret value returned to an agent** through the command line or MCP, including by a server echoing it back.
- A way for an agent to **read files, or send files from disk, over MCP** (file uploads are deliberately unavailable there).
- A request being **sent with an unresolved `{{variable}}`**, or a variable value breaking out of the place it was substituted (for example splitting a query string).
- **Memory-safety or crash bugs** reachable from a malicious server response, HAR file or curl command.

Things that are known and documented, and not vulnerabilities on their own:

- An agent can put `{{name}}` of a remembered secret into a request to **any host**. Plunger can't know which hosts you trust; see the last section of [docs/agents.md](docs/agents.md).
- **Skipping TLS certificate checks** is available on purpose (for local servers) and shows a warning while it's on.
- The Windows executable is **not code-signed yet**, so SmartScreen warns on first run.

## Supported versions

Only the latest release gets security fixes.

## For maintainers

Enable **Private vulnerability reporting** under the repository's *Settings, Code security* so the button above works.

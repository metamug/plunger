# Plunger repo notes for Claude

- Make every HTTP request from this machine with Plunger itself (`target/release/plunger.exe send --url ...` or `plunger curl ...`, or its MCP tools), never raw `curl`. This is the product dogfooding itself; fall back only if Plunger cannot express the request, and then file that as a bug.

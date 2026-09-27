#!/usr/bin/env python3
"""A minimal MCP client that talks to `plunger.exe mcp` over stdio.

Shows the actual protocol traffic an agent like Claude Code exchanges with
Plunger: the initialize handshake, then one tools/call. No SDK, just
line-delimited JSON-RPC, so it doubles as a reference for testing the MCP
server by hand.

Usage: python mcp-demo-client.py <path-to-plunger.exe>
"""
import json
import subprocess
import sys
import time

try:
    sys.stdout.reconfigure(encoding="utf-8")
except Exception:
    pass

PLUNGER = sys.argv[1] if len(sys.argv) > 1 else "plunger.exe"
PAUSE = float(sys.argv[2]) if len(sys.argv) > 2 else 0.9


def say(text=""):
    print(text, flush=True)
    time.sleep(PAUSE)


def rpc(proc, req):
    proc.stdin.write(json.dumps(req) + "\n")
    proc.stdin.flush()


def read_reply(proc):
    line = proc.stdout.readline()
    return json.loads(line) if line else None


def main():
    say("$ type .mcp.json")
    say(json.dumps(
        {"mcpServers": {"plunger": {"command": "plunger.exe", "args": ["mcp"]}}},
        indent=2,
    ))
    say()
    say("Claude Code starts plunger.exe mcp and speaks this over stdio:")
    say()

    proc = subprocess.Popen(
        [PLUNGER, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, text=True, bufsize=1,
    )

    rpc(proc, {
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "claude-code", "version": "demo"},
        },
    })
    read_reply(proc)
    rpc(proc, {"jsonrpc": "2.0", "method": "notifications/initialized"})

    say("-> tools/call  send_request")
    call = {"url": "https://official-joke-api.appspot.com/random_joke"}
    say(json.dumps(call, indent=2))
    say()

    rpc(proc, {
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "send_request", "arguments": call},
    })
    reply = read_reply(proc)
    body = json.loads(reply["result"]["content"][0]["text"])

    say(f"<- {body['status']} {body['status_text']}   {body['elapsed_ms']} ms")
    j = body.get("json") or {}
    say(json.dumps({"setup": j.get("setup"), "punchline": j.get("punchline")}, indent=2))
    say()
    say("Plunger's window just updated live, tagged MCP -- no extra step.")

    proc.stdin.close()
    proc.terminate()


if __name__ == "__main__":
    main()

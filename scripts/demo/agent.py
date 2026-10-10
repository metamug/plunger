"""An AI agent using Plunger over MCP, with pauses so a screen recording can follow it.

Standard library only. Usage: agent.py PLUNGER_BINARY [initial_delay_seconds]
It talks to the fake shop API in scripts/shop-api.py (http://127.0.0.1:18095).
"""
import json
import subprocess
import sys
import time

binary = sys.argv[1]
delay = float(sys.argv[2]) if len(sys.argv) > 2 else 0.0
base = "http://127.0.0.1:18095"

proc = subprocess.Popen([binary, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
counter = 0


def rpc(method, params=None, notification=False):
    global counter
    message = {"jsonrpc": "2.0", "method": method}
    if params is not None:
        message["params"] = params
    if not notification:
        counter += 1
        message["id"] = counter
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()
    if notification:
        return None
    while True:
        reply = json.loads(proc.stdout.readline())
        if reply.get("id") == counter:
            return reply


def call(tool, arguments, pause=1.3):
    rpc("tools/call", {"name": tool, "arguments": arguments})
    time.sleep(pause)


rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "agent", "version": "1"}})
rpc("notifications/initialized", notification=True)
time.sleep(delay)

call("set_variable", {"name": "base", "value": base}, 0.8)
call("set_variable", {"name": "username", "value": "demo"}, 0.5)
call("set_variable", {"name": "password", "value": "demo"}, 0.8)
call(
    "send_request",
    {
        "method": "POST",
        "url": "{{base}}/login",
        "json": {"username": "{{username}}", "password": "{{password}}"},
        "extract": [{"name": "token", "from": "json:$.token"}],
        "select": ["status", "$.user.name"],
    },
)
call("send_request", {"url": "{{base}}/me", "headers": {"Authorization": "Bearer {{token}}"}, "select": ["$.name"]})
call(
    "send_request",
    {
        "method": "POST",
        "url": "{{base}}/orders",
        "headers": {"Authorization": "Bearer {{token}}"},
        "json": {"item_id": 1, "qty": 2},
        "select": ["status", "header:Location"],
    },
)
call("send_request", {"url": "{{base}}/report"}, 0.6)
proc.kill()

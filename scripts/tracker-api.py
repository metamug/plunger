"""A small fake issue tracker (Jira-shaped) for testing agent-style use of Plunger.

Run:  python scripts/tracker-api.py [port]      (default 18096)

Every request needs "Authorization: Bearer tok_tracker".

  POST /issue                    {"fields": {"summary", "description", "assignee"?, "labels"?, "epic"?}}
                                 201 + Location; 400 with a Jira-style message for a body that is not valid
                                 JSON, for an empty assignee name, or for labels that are not an array
  GET  /issue/<key>              the issue; "assignee" is null until set; 404 is an HTML page, not JSON
  GET  /issue/<key>/transitions  the transitions that issue allows
  POST /issue/<key>/transitions  {"transition": {"id"}} -> 204 with no body
  GET  /search?jql=              echoes the jql it received, decoded
  GET  /boom                     500 with an HTML error page
  GET  /old                      301 to /search?jql=moved
  GET  /blob                     a small PNG (binary)
"""
import json
import os
import re
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

TOKEN = "tok_tracker"
ISSUES = {}
TRANSITIONS = [{"id": "11", "name": "To Do"}, {"id": "21", "name": "In Progress"}, {"id": "31", "name": "Done"}]


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def send(self, code, obj=None, raw=None, ctype="application/json", extra=None):
        body = raw if raw is not None else (json.dumps(obj).encode() if obj is not None else b"")
        self.send_response(code)
        if body:
            self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def handle_any(self):
        length = int(self.headers.get("content-length") or 0)
        data = self.rfile.read(length) if length else b""
        url = urlparse(self.path)
        if self.headers.get("Authorization") != f"Bearer {TOKEN}":
            return self.send(401, {"errorMessages": ["You are not authenticated"]})
        path = url.path
        # TRACKER_DELAY=0.6 makes every answer take that long, so a screen recording can show a workflow's
        # steps one after the other
        time.sleep(float(os.environ.get("TRACKER_DELAY", "0")))
        if path == "/boom":
            return self.send(500, raw=b"<html><body>Internal Server Error</body></html>", ctype="text/html")
        if path == "/old":
            return self.send(301, raw=b"", extra={"Location": "/search?jql=moved"})
        if path == "/blob":
            return self.send(200, raw=b"\x89PNG\r\n\x1a\n" + bytes(64), ctype="image/png")
        if path == "/search":
            jql = parse_qs(url.query).get("jql", [""])[0]
            return self.send(200, {"jql": jql, "total": 0})
        if path == "/issue" and self.command == "POST":
            try:
                body = json.loads(data)
            except ValueError as err:
                return self.send(400, {"errorMessages": [f"Illegal unquoted character in the body: {err}"]})
            fields = body.get("fields", {})
            if not isinstance(fields.get("summary"), str) or not fields["summary"]:
                return self.send(400, {"errors": {"summary": "summary is required"}})
            assignee = fields.get("assignee")
            if assignee is not None and not (isinstance(assignee, dict) and assignee.get("name")):
                return self.send(400, {"errors": {"assignee": "assignee needs a name"}})
            labels = fields.get("labels", [])
            if not isinstance(labels, list):
                return self.send(400, {"errors": {"labels": "labels must be an array"}})
            key = f"PRJ-{len(ISSUES) + 1}"
            ISSUES[key] = {
                "key": key,
                "fields": {
                    "summary": fields["summary"],
                    "description": fields.get("description", ""),
                    "assignee": {"displayName": assignee["name"]} if assignee else None,
                    "labels": labels,
                    "epic": fields.get("epic"),
                    "status": {"name": "To Do"},
                },
            }
            return self.send(201, {"id": str(10000 + len(ISSUES)), "key": key}, extra={"Location": f"/issue/{key}"})
        m = re.fullmatch(r"/issue/([A-Z]+-\d+)(/transitions)?", path)
        if m:
            key, sub = m.group(1), m.group(2)
            issue = ISSUES.get(key)
            if issue is None:
                return self.send(404, raw=b"<html><body>Issue does not exist</body></html>", ctype="text/html")
            if sub and self.command == "GET":
                return self.send(200, {"transitions": TRANSITIONS})
            if sub and self.command == "POST":
                wanted = json.loads(data or b"{}").get("transition", {}).get("id")
                found = next((t for t in TRANSITIONS if t["id"] == wanted), None)
                if not found:
                    return self.send(400, {"errorMessages": ["unknown transition"]})
                issue["fields"]["status"] = {"name": found["name"]}
                return self.send(204)
            return self.send(200, issue)
        return self.send(404, {"errorMessages": ["no such endpoint"]})

    do_GET = do_POST = do_PUT = do_DELETE = handle_any

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18096
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()

"""A small fake shop API for trying Plunger (window, CLI and MCP) on a realistic workflow.

Run:  python scripts/shop-api.py [logfile]      (listens on http://127.0.0.1:18095)

  POST /login            {"username":"demo","password":"demo"} -> token + Set-Cookie sid
  GET  /me               needs "Authorization: Bearer tok_abc123"
  GET  /items?page=&per_page=   paginated, Link header
  POST /orders           JSON {"item_id","qty"}; 201 + Location, or 422 with field errors
  GET  /orders/<id>
  GET  /session-only     needs the sid cookie from /login
  GET  /old              301 -> /items?page=1
  GET  /flaky            429 + Retry-After twice, then 200
  GET  /report           a large JSON document (about 3.4 MB)
  GET  /download         binary body with Content-Disposition
  POST /upload           multipart/form-data, reports the parts it received
  GET  /slow             answers after 3 s
"""
import json, re, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

TOKEN = "tok_abc123"
SID = "sess_777"
ITEMS = [{"id": i, "name": f"Widget {i}", "price_cents": 500 + (i * 137) % 900, "stock": (i * 7) % 5} for i in range(1, 13)]
ORDERS = {}
FLAKY = {"n": 0}
LOG = open(sys.argv[1] if len(sys.argv) > 1 else "shop.log", "a", encoding="utf-8")


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def out(self, code, obj=None, raw=None, ctype="application/json", extra=None):
        body = raw if raw is not None else json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def authed(self):
        return self.headers.get("Authorization") == f"Bearer {TOKEN}"

    def handle_any(self):
        n = int(self.headers.get("content-length") or 0)
        data = self.rfile.read(n) if n else b""
        u = urlparse(self.path)
        q = parse_qs(u.query)
        LOG.write(f"{self.command} {self.path} auth={self.headers.get('Authorization')} cookie={self.headers.get('Cookie')} ct={self.headers.get('Content-Type')} len={n}\n")
        LOG.flush()
        p = u.path
        if p == "/login" and self.command == "POST":
            try:
                body = json.loads(data or b"{}")
            except Exception:
                return self.out(400, {"error": "body must be JSON"})
            if body.get("username") == "demo" and body.get("password") == "demo":
                return self.out(200, {"token": TOKEN, "expires_in": 3600, "user": {"id": 7, "name": "Demo User"}},
                                extra={"Set-Cookie": f"sid={SID}; HttpOnly; Path=/"})
            return self.out(401, {"error": "invalid credentials"})
        if p == "/me":
            return self.out(200, {"id": 7, "name": "Demo User"}) if self.authed() else self.out(401, {"error": "missing or invalid token"})
        if p == "/items":
            page = int(q.get("page", ["1"])[0])
            per = int(q.get("per_page", ["5"])[0])
            chunk = ITEMS[(page - 1) * per: page * per]
            nxt = page + 1 if page * per < len(ITEMS) else None
            extra = {"Link": f'</items?page={nxt}&per_page={per}>; rel="next"'} if nxt else {}
            return self.out(200, {"items": chunk, "page": page, "total": len(ITEMS), "next_page": nxt}, extra=extra)
        if p == "/orders" and self.command == "POST":
            if not self.authed():
                return self.out(401, {"error": "missing or invalid token"})
            try:
                body = json.loads(data or b"{}")
            except Exception:
                return self.out(400, {"error": "body must be JSON", "content_type": self.headers.get("Content-Type")})
            errs = []
            if not isinstance(body.get("item_id"), int) or not any(i["id"] == body.get("item_id") for i in ITEMS):
                errs.append({"field": "item_id", "message": "unknown item"})
            if not isinstance(body.get("qty"), int) or body.get("qty", 0) < 1:
                errs.append({"field": "qty", "message": "must be >= 1"})
            if errs:
                return self.out(422, {"errors": errs})
            oid = len(ORDERS) + 1
            ORDERS[oid] = {"id": oid, "item_id": body["item_id"], "qty": body["qty"], "status": "pending"}
            return self.out(201, ORDERS[oid], extra={"Location": f"/orders/{oid}"})
        m = re.fullmatch(r"/orders/(\d+)", p)
        if m:
            if not self.authed():
                return self.out(401, {"error": "missing or invalid token"})
            o = ORDERS.get(int(m.group(1)))
            return self.out(200, o) if o else self.out(404, {"error": "no such order"})
        if p == "/session-only":
            ok = f"sid={SID}" in (self.headers.get("Cookie") or "")
            return self.out(200, {"hello": "cookie session"}) if ok else self.out(401, {"error": "no session cookie"})
        if p == "/old":
            return self.out(301, {"moved": True}, extra={"Location": "/items?page=1"})
        if p == "/report":
            rows = [{"order": i, "note": "x" * 40, "amount": i * 3} for i in range(40000)]
            return self.out(200, {"generated": "now", "rows": rows})
        if p == "/flaky":
            FLAKY["n"] += 1
            if FLAKY["n"] <= 2:
                return self.out(429, {"error": "slow down"}, extra={"Retry-After": "1"})
            return self.out(200, {"ok": True, "after_attempts": FLAKY["n"]})
        if p == "/download":
            return self.out(200, raw=bytes(range(256)) * 40, ctype="image/png", extra={"Content-Disposition": 'attachment; filename="logo.png"'})
        if p == "/upload" and self.command == "POST":
            ct = self.headers.get("Content-Type", "")
            names = re.findall(rb'name="([^"]+)"(?:; filename="([^"]*)")?', data)
            return self.out(200, {"content_type": ct.split(";")[0], "bytes": len(data), "parts": [[a.decode(), b.decode()] for a, b in names]})
        if p == "/slow":
            time.sleep(3)
            return self.out(200, {"slow": True})
        return self.out(404, {"error": "no such path", "path": p})

    do_GET = do_POST = do_PUT = do_DELETE = do_PATCH = handle_any

    def log_message(self, *a):
        pass


ThreadingHTTPServer(("127.0.0.1", 18095), H).serve_forever()

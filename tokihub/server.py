"""A small HTTP server on 127.0.0.1 that serves the UI and lets it call the Hub.

The same server is used whether the hub shows in its own window (pywebview) or a browser tab.

Safety: the API is only reachable from the hub's own page. Every API call must carry the
random token that is written into that page, in a custom header; other websites cannot read
the page or add that header without the server's permission (which it never gives), and the
Host check stops DNS-rebinding tricks. Files are only served from ui/ and catalog images.
"""
import hmac
import json
import logging
import mimetypes
import secrets
import time
import urllib.parse
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from tokihub.catalog import IMAGE_TYPES
from tokihub.errors import HubError
from tokihub.paths import is_within

log = logging.getLogger(__name__)

API_METHODS = frozenset({
    "state", "poll", "reload",
    "install", "cancel", "uninstall", "launch", "stop",
    "open_folder", "open_log", "open_place",
    "save_settings", "clear_token", "pick_folder",
})
MAX_BODY = 1024 * 1024
TOKEN_PLACEHOLDER = "__TOKI_TOKEN__"
# Spelled out because on Windows `mimetypes` reads the registry, which other programs can change.
CONTENT_TYPES = {".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".woff2": "font/woff2",
                 ".svg": "image/svg+xml", ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
                 ".gif": "image/gif", ".webp": "image/webp", ".ico": "image/x-icon", ".md": "text/plain"}
CSP = ("default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; "
       "font-src 'self'; connect-src 'self'; frame-ancestors 'none'")


class HubServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = False

    def __init__(self, hub, ui_dir, port=0):
        super().__init__(("127.0.0.1", port), Handler)
        self.hub = hub
        self.ui_dir = Path(ui_dir).resolve()
        self.token = secrets.token_urlsafe(32)
        self.last_api_call = None       # time.monotonic() of the page's latest call

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server_port}/"


class Handler(BaseHTTPRequestHandler):
    server_version = "TokiHub"
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        log.debug("%s - %s", self.address_string(), fmt % args)

    # -- Plumbing --

    def _host_ok(self):
        port = self.server.server_port
        return self.headers.get("Host", "") in (f"127.0.0.1:{port}", f"localhost:{port}")

    def _send(self, status, body, content_type, extra=None):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Cache-Control", "no-store")
        for key, value in (extra or {}).items():
            self.send_header(key, value)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _send_json(self, status, payload):
        self._send(status, json.dumps(payload).encode("utf-8"), "application/json; charset=utf-8")

    def _fail(self, status):
        # The request body may not have been read, so this connection cannot be reused.
        self.close_connection = True
        self._send(status, status.phrase.encode(), "text/plain; charset=utf-8", {"Connection": "close"})

    # -- GET: the page, its files, and catalog images --

    def do_GET(self):
        if not self._host_ok():
            return self._fail(HTTPStatus.FORBIDDEN)
        path = urllib.parse.unquote(urllib.parse.urlsplit(self.path).path)
        if path in ("/", "/index.html"):
            return self._send_index()
        if path.startswith("/catalog/"):
            game_id, _, rest = path[len("/catalog/"):].partition("/")
            game = self.server.hub.catalog.games.get(game_id)
            if game is None or Path(rest).suffix.lower() not in IMAGE_TYPES:
                return self._fail(HTTPStatus.NOT_FOUND)
            folder = game.folder
            file = folder / rest
        else:
            folder = self.server.ui_dir
            file = folder / path.lstrip("/")
        if not is_within(file, folder) or not file.is_file():
            return self._fail(HTTPStatus.NOT_FOUND)
        content_type = CONTENT_TYPES.get(file.suffix.lower()) or mimetypes.guess_type(file.name)[0] \
            or "application/octet-stream"
        self._send(HTTPStatus.OK, file.read_bytes(), content_type)

    do_HEAD = do_GET

    def _send_index(self):
        page = (self.server.ui_dir / "index.html").read_text(encoding="utf-8")
        page = page.replace(TOKEN_PLACEHOLDER, self.server.token)
        self._send(HTTPStatus.OK, page.encode("utf-8"), "text/html; charset=utf-8",
                   {"Content-Security-Policy": CSP, "Referrer-Policy": "no-referrer"})

    # -- POST /api/<method>: {"args": [...]} -> {"ok": true, "result": ...} --

    def do_POST(self):
        if not self._host_ok():
            return self._fail(HTTPStatus.FORBIDDEN)
        token = self.headers.get("X-Toki-Token", "")
        if not hmac.compare_digest(token.encode(), self.server.token.encode()):
            return self._fail(HTTPStatus.FORBIDDEN)
        path = urllib.parse.urlsplit(self.path).path
        name = path[len("/api/"):] if path.startswith("/api/") else ""
        if name not in API_METHODS:
            return self._fail(HTTPStatus.NOT_FOUND)
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return self._fail(HTTPStatus.BAD_REQUEST)
        if length > MAX_BODY:
            return self._fail(HTTPStatus.REQUEST_ENTITY_TOO_LARGE)
        try:
            body = json.loads(self.rfile.read(length) or b"{}")
            args = body.get("args", []) if isinstance(body, dict) else None
        except ValueError:
            args = None
        if not isinstance(args, list):
            return self._fail(HTTPStatus.BAD_REQUEST)

        self.server.last_api_call = time.monotonic()
        try:
            result = getattr(self.server.hub, name)(*args)
        except HubError as e:
            return self._send_json(HTTPStatus.OK, {"ok": False, "error": str(e)})
        except TypeError as e:
            if "positional argument" in str(e):
                return self._send_json(HTTPStatus.BAD_REQUEST, {"ok": False, "error": f"Bad call to {name}."})
            log.exception("API %s failed", name)
            return self._send_json(HTTPStatus.INTERNAL_SERVER_ERROR, {"ok": False, "error": f"Unexpected error: {e}"})
        except Exception as e:
            log.exception("API %s failed", name)
            return self._send_json(HTTPStatus.INTERNAL_SERVER_ERROR, {"ok": False, "error": f"Unexpected error: {e}"})
        self._send_json(HTTPStatus.OK, {"ok": True, "result": result})

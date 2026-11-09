import json
import os
import shutil
import sys
import tempfile
import threading
import time
import unittest
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest import mock

from tokihub.hub import Hub


def wait_for(predicate, timeout=15, message="condition"):
    deadline = time.time() + timeout
    while time.time() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {message}")


def make_zip(path, files, root=None):
    """files: {relative name: str or bytes}. root: wrap everything in this top folder."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w") as archive:
        for name, content in files.items():
            archive.writestr(f"{root}/{name}" if root else name, content)
    return path


def write_game(catalog_dir, game_id, data):
    folder = Path(catalog_dir) / game_id
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "game.json").write_text(json.dumps(data) if not isinstance(data, str) else data, encoding="utf-8")
    return folder


def game_with(versions, title="Test Game"):
    return {"title": title, "versions": versions}


def version_with(version, source, launch=None, platform="any"):
    return {"version": version, "builds": [{"platform": platform, "source": source,
                                            "launch": launch or {"type": "python", "path": "main.py"}}]}


class FakeWeb:
    """A tiny web server for tests. `routes` maps a path to a function(handler) -> (status, headers, body)."""

    def __init__(self):
        self.routes = {}
        self.requests = []
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                fake.requests.append((self.path, dict(self.headers)))
                route = fake.routes.get(self.path)
                if route is None:
                    self.send_error(404)
                    return
                result = route(self)
                if result is None:      # the route wrote the response itself
                    return
                status, headers, body = result
                self.send_response(status)
                for key, value in headers.items():
                    self.send_header(key, value)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.base = f"http://127.0.0.1:{self.server.server_port}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def file(self, path, data, content_type="application/octet-stream"):
        self.routes[path] = lambda handler: (200, {"Content-Type": content_type}, data)

    def json(self, path, payload, status=200):
        body = json.dumps(payload).encode()
        self.routes[path] = lambda handler: (status, {"Content-Type": "application/json"}, body)

    def redirect(self, path, to):
        self.routes[path] = lambda handler: (302, {"Location": self.base + to}, b"")

    def headers_for(self, path):
        return [headers for p, headers in self.requests if p == path]

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class HubTestCase(unittest.TestCase):
    """Gives each test a fresh catalog folder, data folder and fake web server."""

    platform = "windows"

    def setUp(self):
        # A token in the real environment would leak into requests, so hide it from every test.
        environment = mock.patch.dict(os.environ)
        environment.start()
        self.addCleanup(environment.stop)
        for name in ("GITHUB_TOKEN", "GH_TOKEN", "TOKIHUB_DATA_DIR"):
            os.environ.pop(name, None)
        self.tmp = Path(tempfile.mkdtemp(prefix="tokihub-test-"))
        self.catalog_dir = self.tmp / "catalog"
        self.catalog_dir.mkdir()
        self.data_dir = self.tmp / "data"
        self.web = FakeWeb()

    def tearDown(self):
        self.web.close()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def hub(self):
        hub = Hub(catalog_dir=self.catalog_dir, data_dir=self.data_dir, github_api=self.web.base,
                  platform=self.platform)
        hub.settings.update({"python": sys.executable})
        return hub

    def install_and_wait(self, hub, game_id, version):
        task = hub.install(game_id, version)
        return wait_for(lambda: next((t for t in hub.installer.tasks()
                                      if t["id"] == task["id"] and t["status"] not in ("queued", "downloading", "installing")),
                                     None), message=f"install of {game_id} {version}")

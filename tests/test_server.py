import http.client
import json
import threading
import unittest

from tests.helpers import HubTestCase
from tokihub import paths
from tokihub.hub import Hub
from tokihub.server import HubServer


class ServerTests(HubTestCase):
    def setUp(self):
        super().setUp()
        hub = Hub(catalog_dir=paths.catalog_dir(), data_dir=self.data_dir, github_api=self.web.base)
        self.server = HubServer(hub, paths.ui_dir())
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.port = self.server.server_port

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        super().tearDown()

    def request(self, method, path, body=None, headers=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        connection.request(method, path, body=body, headers=headers or {})
        response = connection.getresponse()
        data = response.read()
        connection.close()
        return response, data

    def call(self, name, *args, token=None, host=None):
        headers = {"Content-Type": "application/json", "X-Toki-Token": token or self.server.token}
        if host:
            headers["Host"] = host
        response, data = self.request("POST", f"/api/{name}", json.dumps({"args": list(args)}), headers)
        return response.status, json.loads(data) if data.startswith(b"{") else data

    def test_page_carries_the_token_and_a_csp(self):
        response, page = self.request("GET", "/")
        self.assertEqual(response.status, 200)
        self.assertIn(self.server.token.encode(), page)
        self.assertNotIn(b"__TOKI_TOKEN__", page)
        self.assertIn("script-src 'self'", response.getheader("Content-Security-Policy"))

    def test_static_files_and_catalog_images(self):
        for path, kind in (("/css/hub.css", "text/css"), ("/js/hub.js", "text/javascript"),
                           ("/fonts/exo-2-latin-wght-italic.woff2", "font/woff2"),
                           ("/catalog/hu-taong/icon.png", "image/png")):
            with self.subTest(path=path):
                response, _ = self.request("GET", path)
                self.assertEqual(response.status, 200)
                self.assertEqual(response.getheader("Content-Type"), kind)

    def test_nothing_else_is_served(self):
        for path in ("/catalog/hu-taong/game.json", "/../tokihub/hub.py", "/%2e%2e/tokihub/hub.py",
                     "/catalog/../tokihub/hub.py", "/catalog/hu-taong/../../ui/img/halo.svg",
                     "/catalog/no-such-game/icon.png", "/nope.html"):
            with self.subTest(path=path):
                self.assertEqual(self.request("GET", path)[0].status, 404)

    def test_api_needs_the_token(self):
        self.assertEqual(self.call("state", token="wrong")[0], 403)
        status, payload = self.call("state")
        self.assertEqual(status, 200)
        self.assertTrue(payload["ok"])
        self.assertEqual(payload["result"]["games"][0]["title"], "Hu Taong")
        self.assertNotIn("github_token", json.dumps(payload), "the token must never reach the page")

    def test_foreign_host_header_is_refused(self):
        self.assertEqual(self.call("state", host=f"evil.example:{self.port}")[0], 403)
        response, _ = self.request("GET", "/", headers={"Host": "evil.example"})
        self.assertEqual(response.status, 403)

    def test_only_listed_methods_are_callable(self):
        self.assertEqual(self.call("__init__")[0], 404)
        self.assertEqual(self.call("_game_json")[0], 404)

    def test_hub_errors_come_back_as_messages(self):
        status, payload = self.call("install", "no-such-game", "1.0")
        self.assertEqual(status, 200)
        self.assertEqual(payload, {"ok": False, "error": "There is no game 'no-such-game' in the catalog."})
        self.assertEqual(self.call("install")[0], 400)

    def test_settings_round_trip_without_exposing_the_token(self):
        status, payload = self.call("save_settings", {"github_token": "ghp_secret"})
        self.assertTrue(payload["result"]["has_token"])
        self.assertNotIn("ghp_secret", json.dumps(payload))
        _, payload = self.call("save_settings", {"install_dir": "relative/path"})
        self.assertFalse(payload["ok"])
        _, payload = self.call("clear_token")
        self.assertFalse(payload["result"]["has_token"])


if __name__ == "__main__":
    unittest.main()

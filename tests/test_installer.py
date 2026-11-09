import hashlib
import io
import os
import time
import unittest
import zipfile

from tests.helpers import HubTestCase, game_with, make_zip, version_with, wait_for, write_game
from tokihub.errors import HubError
from tokihub.installer import MARKER

GAME_FILES = {"main.py": "print('hello from the game')\n", "Resources/data.txt": "data"}


def zip_bytes(files, root=None):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, content in files.items():
            archive.writestr(f"{root}/{name}" if root else name, content)
    return buffer.getvalue()


class UrlInstallTests(HubTestCase):
    def add(self, source, launch=None, version="1.0"):
        write_game(self.catalog_dir, "game", game_with([version_with(version, source, launch)]))

    def test_zip_with_a_top_folder_installs_flat(self):
        self.web.file("/game.zip", zip_bytes(GAME_FILES, root="Game-1.0"))
        self.add({"type": "url", "url": f"{self.web.base}/game.zip"})
        hub = self.hub()
        task = self.install_and_wait(hub, "game", "1.0")
        self.assertEqual(task["status"], "done", task["error"])
        folder = hub.installer.version_dir("game", "1.0")
        self.assertTrue((folder / "main.py").is_file())
        self.assertTrue((folder / "Resources" / "data.txt").is_file())
        self.assertTrue((folder / MARKER).is_file())
        installed = hub.poll()["installed"]["game"]["1.0"]
        self.assertGreater(installed["size"], 0)
        self.assertEqual(list(hub.data_dir.joinpath("downloads").iterdir()), [], "temp download left behind")

    def test_matching_checksum_installs(self):
        data = zip_bytes(GAME_FILES)
        self.web.file("/game.zip", data)
        self.add({"type": "url", "url": f"{self.web.base}/game.zip", "sha256": hashlib.sha256(data).hexdigest()})
        self.assertEqual(self.install_and_wait(self.hub(), "game", "1.0")["status"], "done")

    def test_wrong_checksum_installs_nothing(self):
        self.web.file("/game.zip", zip_bytes(GAME_FILES))
        self.add({"type": "url", "url": f"{self.web.base}/game.zip", "sha256": "0" * 64})
        hub = self.hub()
        task = self.install_and_wait(hub, "game", "1.0")
        self.assertEqual(task["status"], "error")
        self.assertIn("does not match the sha256", task["error"])
        self.assertFalse(hub.installer.version_dir("game", "1.0").exists())
        self.assertEqual(list((hub.data_dir / "downloads").iterdir()), [], "partial download left behind")
        game_folder = hub.settings.install_dir / "game"
        self.assertEqual(list(game_folder.iterdir()) if game_folder.exists() else [], [], "staging left behind")

    def test_zip_slip_is_refused(self):
        self.web.file("/evil.zip", zip_bytes({"main.py": "", "../../escaped.txt": "gotcha"}))
        self.add({"type": "url", "url": f"{self.web.base}/evil.zip"})
        task = self.install_and_wait(self.hub(), "game", "1.0")
        self.assertEqual(task["status"], "error")
        self.assertIn("unsafe path", task["error"])
        self.assertFalse(any(self.tmp.rglob("escaped.txt")))

    def test_archive_without_the_launch_file_is_refused(self):
        self.web.file("/game.zip", zip_bytes({"readme.txt": "no game here"}))
        self.add({"type": "url", "url": f"{self.web.base}/game.zip"})
        task = self.install_and_wait(self.hub(), "game", "1.0")
        self.assertEqual(task["status"], "error")
        self.assertIn("do not contain 'main.py'", task["error"])

    def test_single_file_download_is_kept_as_is(self):
        self.web.file("/Game.exe", b"#!/bin/sh\necho hi\n")
        self.add({"type": "url", "url": f"{self.web.base}/Game.exe"}, {"type": "executable", "path": "Game.exe"})
        hub = self.hub()
        self.assertEqual(self.install_and_wait(hub, "game", "1.0")["status"], "done")
        exe = hub.installer.version_dir("game", "1.0") / "Game.exe"
        self.assertTrue(exe.is_file())
        if os.name != "nt":
            self.assertTrue(os.access(exe, os.X_OK))

    def test_http_error_is_explained(self):
        self.add({"type": "url", "url": f"{self.web.base}/missing.zip"})
        task = self.install_and_wait(self.hub(), "game", "1.0")
        self.assertEqual(task["status"], "error")
        self.assertIn("HTTP 404", task["error"])

    def test_cancel_stops_the_download(self):
        def slow(handler):
            handler.send_response(200)
            handler.send_header("Content-Length", str(64 * 1024 * 200))
            handler.end_headers()
            try:
                for _ in range(200):
                    handler.wfile.write(b"x" * 64 * 1024)
                    time.sleep(0.02)
            except OSError:
                pass
        self.web.routes["/big.zip"] = slow
        self.add({"type": "url", "url": f"{self.web.base}/big.zip"})
        hub = self.hub()
        task = hub.install("game", "1.0")
        wait_for(lambda: hub.installer.tasks()[0]["received"] > 0, message="download to start")
        with self.assertRaises(HubError):
            hub.install("game", "1.0")      # already downloading
        self.assertTrue(hub.cancel(task["id"]))
        done = wait_for(lambda: next((t for t in hub.installer.tasks() if t["status"] == "cancelled"), None),
                        message="cancel")
        self.assertEqual(done["id"], task["id"])
        self.assertFalse(hub.installer.version_dir("game", "1.0").exists())
        self.assertEqual(list((hub.data_dir / "downloads").iterdir()), [], "partial download left behind")

    def test_reinstall_and_uninstall(self):
        self.web.file("/game.zip", zip_bytes(GAME_FILES))
        self.add({"type": "url", "url": f"{self.web.base}/game.zip"})
        hub = self.hub()
        self.install_and_wait(hub, "game", "1.0")
        folder = hub.installer.version_dir("game", "1.0")
        (folder / "leftover.txt").write_text("from the old install")
        self.assertEqual(self.install_and_wait(hub, "game", "1.0")["status"], "done")
        self.assertFalse((folder / "leftover.txt").exists(), "reinstall should replace the old files")
        hub.uninstall("game", "1.0")
        self.assertFalse(folder.exists())
        self.assertFalse(folder.parent.exists(), "empty game folder should be tidied away")
        self.assertEqual(hub.poll()["installed"], {})
        with self.assertRaises(HubError):
            hub.uninstall("game", "1.0")

    def test_interrupted_installs_are_ignored_then_swept(self):
        self.web.file("/game.zip", zip_bytes(GAME_FILES))
        self.add({"type": "url", "url": f"{self.web.base}/game.zip"})
        hub = self.hub()
        self.install_and_wait(hub, "game", "1.0")
        staging = hub.settings.install_dir / "game" / ".staging-1.0-t999"
        staging.mkdir()
        (staging / MARKER).write_text((hub.installer.version_dir("game", "1.0") / MARKER).read_text())
        part = hub.data_dir / "downloads" / "t999-game.zip.part"
        part.write_bytes(b"half")
        self.assertEqual(hub.poll()["installed"]["game"]["1.0"]["path"], str(hub.installer.version_dir("game", "1.0")))
        old = time.time() - 2 * 3600
        for path in (staging, part):
            os.utime(path, (old, old))
        self.hub()      # a fresh start sweeps leftovers older than an hour
        self.assertFalse(staging.exists())
        self.assertFalse(part.exists())
        self.assertTrue(hub.installer.version_dir("game", "1.0").exists())

    def test_versions_install_side_by_side(self):
        self.web.file("/game.zip", zip_bytes(GAME_FILES))
        source = {"type": "url", "url": f"{self.web.base}/game.zip"}
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", source), version_with("2.0-beta", source)]))
        hub = self.hub()
        self.install_and_wait(hub, "game", "1.0")
        self.install_and_wait(hub, "game", "2.0-beta")
        self.assertEqual(set(hub.poll()["installed"]["game"]), {"1.0", "2.0-beta"})

    def test_no_build_for_this_platform(self):
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", {"type": "url", "url": "https://x/y.zip"},
                                                                     platform="macos")]))
        with self.assertRaises(HubError) as raised:
            self.hub().install("game", "1.0")
        self.assertIn("no build for windows", str(raised.exception))


class LocalInstallTests(HubTestCase):
    def test_local_folder_is_copied(self):
        source = self.tmp / "my-build"
        (source / ".git").mkdir(parents=True)
        (source / "main.py").write_text("print('hi')")
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", {"type": "local", "path": str(source)})]))
        hub = self.hub()
        self.assertEqual(self.install_and_wait(hub, "game", "1.0")["status"], "done")
        folder = hub.installer.version_dir("game", "1.0")
        self.assertTrue((folder / "main.py").is_file())
        self.assertFalse((folder / ".git").exists())

    def test_local_zip_relative_to_the_game_folder(self):
        folder = write_game(self.catalog_dir, "game", game_with([version_with("1.0", {"type": "local", "path": "builds/1.0.zip"})]))
        make_zip(folder / "builds" / "1.0.zip", GAME_FILES)
        self.assertEqual(self.install_and_wait(self.hub(), "game", "1.0")["status"], "done")

    def test_missing_local_files_are_explained(self):
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", {"type": "local", "path": "C:/nowhere"})]))
        task = self.install_and_wait(self.hub(), "game", "1.0")
        self.assertEqual(task["status"], "error")
        self.assertIn("not on this computer", task["error"])


class GitHubInstallTests(HubTestCase):
    def test_release_asset_from_a_private_repo(self):
        self.web.json("/repos/Owner/Game/releases/tags/v1.0",
                      {"assets": [{"name": "Game-windows.zip", "url": f"{self.web.base}/assets/7"}]})
        self.web.redirect("/assets/7", "/storage/signed.zip")
        self.web.file("/storage/signed.zip", zip_bytes(GAME_FILES, root="Game"))
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", {
            "type": "github-release", "repo": "Owner/Game", "tag": "v1.0", "asset": "Game-windows.zip"})]))
        hub = self.hub()
        hub.save_settings({"github_token": "secret-token"})
        task = self.install_and_wait(hub, "game", "1.0")
        self.assertEqual(task["status"], "done", task["error"])

        api_headers = self.web.headers_for("/repos/Owner/Game/releases/tags/v1.0")[0]
        self.assertEqual(api_headers.get("Authorization"), "Bearer secret-token")
        asset_headers = self.web.headers_for("/assets/7")[0]
        self.assertEqual(asset_headers.get("Authorization"), "Bearer secret-token")
        self.assertEqual(asset_headers.get("Accept"), "application/octet-stream")
        storage_headers = self.web.headers_for("/storage/signed.zip")[0]
        self.assertNotIn("Authorization", storage_headers, "token must not be sent to the redirect target")

    def test_missing_asset_lists_what_the_release_has(self):
        self.web.json("/repos/Owner/Game/releases/tags/v1.0", {"assets": [{"name": "Other.zip", "url": "x"}]})
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", {
            "type": "github-release", "repo": "Owner/Game", "tag": "v1.0", "asset": "Game.zip"})]))
        task = self.install_and_wait(self.hub(), "game", "1.0")
        self.assertIn("no file named 'Game.zip' (it has: Other.zip)", task["error"])

    def test_not_found_suggests_a_token(self):
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", {
            "type": "github-release", "repo": "Owner/Private", "tag": "v1.0", "asset": "Game.zip"})]))
        task = self.install_and_wait(self.hub(), "game", "1.0")
        self.assertIn("If the repository is private, add a GitHub token", task["error"])

    def test_source_archive(self):
        ref = "db6da17c8519a61731f6cffb39da2e2cd2730935"
        self.web.redirect(f"/repos/Owner/Game/zipball/{ref}", "/codeload/archive.zip")
        self.web.file("/codeload/archive.zip", zip_bytes(GAME_FILES, root=f"Owner-Game-{ref[:7]}"))
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", {
            "type": "github-archive", "repo": "Owner/Game", "ref": ref})]))
        hub = self.hub()
        task = self.install_and_wait(hub, "game", "1.0")
        self.assertEqual(task["status"], "done", task["error"])
        self.assertTrue((hub.installer.version_dir("game", "1.0") / "main.py").is_file())
        self.assertNotIn("Authorization", self.web.headers_for(f"/repos/Owner/Game/zipball/{ref}")[0],
                         "no token configured, so none should be sent")


if __name__ == "__main__":
    unittest.main()

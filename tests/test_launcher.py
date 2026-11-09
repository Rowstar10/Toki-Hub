import os
import unittest

from tests.helpers import HubTestCase, game_with, version_with, wait_for, write_game
from tokihub.errors import HubError


class LaunchTests(HubTestCase):
    def install_local(self, files, launch=None, version="1.0"):
        source = self.tmp / f"build-{version}"
        source.mkdir(parents=True, exist_ok=True)
        for name, content in files.items():
            (source / name).write_text(content)
        write_game(self.catalog_dir, "game", game_with([version_with(version, {"type": "local", "path": str(source)}, launch)]))
        hub = self.hub()
        self.assertEqual(self.install_and_wait(hub, "game", version)["status"], "done")
        return hub

    def wait_exit(self, hub):
        return wait_for(lambda: next(iter(hub.poll()["events"]), None), message="game to exit")

    def test_python_game_runs_and_is_logged(self):
        hub = self.install_local({"main.py": "import os\nprint('hello from', os.path.basename(os.getcwd()))\n"})
        result = hub.launch("game", "1.0")
        self.assertTrue(result["tracked"])
        event = self.wait_exit(hub)
        self.assertEqual((event["type"], event["code"], event["stopped"], event["log_tail"]), ("exited", 0, False, ""))
        log = hub.launcher.log_path("game", "1.0").read_text()
        self.assertIn("hello from 1.0", log, "game should run inside its own folder")
        self.assertEqual(hub.poll()["running"], [])

    def test_crash_reports_the_end_of_the_log(self):
        hub = self.install_local({"main.py": "import sys\nprint('No module named pygame', file=sys.stderr)\nsys.exit(3)\n"})
        hub.launch("game", "1.0")
        event = self.wait_exit(hub)
        self.assertEqual(event["code"], 3)
        self.assertIn("No module named pygame", event["log_tail"])
        self.assertEqual(hub.poll(event["id"])["events"], [], "events are only delivered once per cursor")

    def test_one_instance_per_version_and_stop(self):
        hub = self.install_local({"main.py": "import time\ntime.sleep(30)\n"})
        hub.launch("game", "1.0")
        self.assertEqual(len(hub.poll()["running"]), 1)
        with self.assertRaises(HubError):
            hub.launch("game", "1.0")
        with self.assertRaises(HubError):
            hub.uninstall("game", "1.0")
        with self.assertRaises(HubError):
            hub.install("game", "1.0")
        hub.stop("game", "1.0")
        event = self.wait_exit(hub)
        self.assertTrue(event["stopped"], "a stop the player asked for is not a crash")
        self.assertEqual(event["log_tail"], "")

    @unittest.skipIf(os.name == "nt", "shell scripts are a POSIX thing")
    def test_executable_game(self):
        hub = self.install_local({"run.sh": "#!/bin/sh\necho native game\n"}, {"type": "executable", "path": "run.sh"})
        hub.launch("game", "1.0")
        self.assertEqual(self.wait_exit(hub)["code"], 0)
        self.assertIn("native game", hub.launcher.log_path("game", "1.0").read_text())

    def test_missing_python_is_explained(self):
        hub = self.install_local({"main.py": ""})
        hub.settings.update({"python": str(self.tmp / "no-such-python")})
        with self.assertRaises(HubError) as raised:
            hub.launch("game", "1.0")
        self.assertIn("Set the Python interpreter in Settings", str(raised.exception))

    def test_not_installed(self):
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", {"type": "url", "url": "https://x/y.zip"})]))
        with self.assertRaises(HubError):
            self.hub().launch("game", "1.0")


if __name__ == "__main__":
    unittest.main()

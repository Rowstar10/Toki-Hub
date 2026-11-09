import contextlib
import io
import json

from tests.helpers import HubTestCase, write_game
from tokihub.catalog import Catalog
from tokihub.cli import main


class CliTests(HubTestCase):
    def run_cli(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            code = main(["--catalog", str(self.catalog_dir), *args])
        return code, out.getvalue()

    def test_new_game_makes_a_valid_entry(self):
        code, output = self.run_cli("new-game", "Toki's Big Adventure!")
        self.assertEqual(code, 0, output)
        game_json = self.catalog_dir / "toki-s-big-adventure" / "game.json"
        self.assertEqual(json.loads(game_json.read_text())["title"], "Toki's Big Adventure!")
        catalog = Catalog.load(self.catalog_dir)
        self.assertEqual(catalog.problems, [])
        self.assertIn("toki-s-big-adventure", catalog.games)
        self.assertEqual(self.run_cli("new-game", "Toki's Big Adventure!")[0], 1, "should not overwrite")

    def test_check_reports_problems(self):
        write_game(self.catalog_dir, "good", {"title": "Good", "versions": []})
        self.assertEqual(self.run_cli("check")[0], 0)
        write_game(self.catalog_dir, "bad", {"title": "Bad", "versions": [{"version": "1.0", "builds": [{}]}]})
        code, output = self.run_cli("check")
        self.assertEqual(code, 1)
        self.assertIn("bad/game.json: version 1.0: builds[0]: source: is required", output)

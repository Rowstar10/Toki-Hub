import unittest

from tests.helpers import HubTestCase, game_with, version_with, write_game
from tokihub import paths
from tokihub.catalog import Catalog
from tokihub.versions import newest_first, version_key

GOOD_SOURCE = {"type": "url", "url": "https://example.com/game.zip"}


class VersionOrderTests(unittest.TestCase):
    def test_newest_first(self):
        ordered = newest_first(["1.0", "1.10", "1.9", "2.0-beta", "2.0", "0.9", "nightly", "2.0-rc1"])
        self.assertEqual(ordered, ["2.0", "2.0-rc1", "2.0-beta", "1.10", "1.9", "1.0", "0.9", "nightly"])

    def test_equivalent_spellings(self):
        self.assertEqual(version_key("1.0"), version_key("1.0.0"))
        self.assertEqual(version_key("v1.2"), version_key("1.2"))


class RealCatalogTests(unittest.TestCase):
    def test_shipped_catalog_has_no_problems(self):
        catalog = Catalog.load(paths.catalog_dir())
        self.assertEqual(catalog.problems, [])
        hu_taong = catalog.game("hu-taong")
        self.assertEqual(hu_taong.title, "Hu Taong")
        self.assertTrue(hu_taong.icon.is_file() and hu_taong.cover.is_file())
        build = hu_taong.version("1.0").build_for("windows")
        self.assertEqual(build.source["type"], "github-archive")
        self.assertEqual(build.launch, {"type": "python", "path": "main.py", "args": []})


class CatalogValidationTests(HubTestCase):
    def load(self):
        return Catalog.load(self.catalog_dir)

    def test_versions_are_sorted_newest_first(self):
        write_game(self.catalog_dir, "game", game_with([version_with(v, GOOD_SOURCE) for v in ("1.0", "1.2", "1.1")]))
        self.assertEqual([v.version for v in self.load().game("game").versions], ["1.2", "1.1", "1.0"])

    def test_invalid_json_is_reported_with_its_position(self):
        write_game(self.catalog_dir, "broken", '{"title": "Broken",, }')
        catalog = self.load()
        self.assertEqual(catalog.games, {})
        self.assertIn("broken/game.json: is not valid JSON (line 1", catalog.problems[0])

    def test_missing_title_skips_the_game(self):
        write_game(self.catalog_dir, "untitled", {"versions": []})
        catalog = self.load()
        self.assertEqual(catalog.games, {})
        self.assertIn("'title' is required", catalog.problems[0])

    def test_bad_folder_name_is_reported(self):
        write_game(self.catalog_dir, "My Game", game_with([]))
        self.assertIn("folder name must be lowercase", self.load().problems[0])

    def test_unknown_source_type_drops_only_that_build(self):
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", {"type": "ftp", "url": "x"})]))
        catalog = self.load()
        version = catalog.game("game").version("1.0")
        self.assertEqual(version.builds, [])
        self.assertIn("'type' must be one of url, github-release, github-archive, local", catalog.problems[0])

    def test_unknown_fields_are_warned_about_but_kept(self):
        entry = version_with("1.0", GOOD_SOURCE)
        entry["builds"][0]["lauch"] = {}
        write_game(self.catalog_dir, "game", game_with([entry]))
        catalog = self.load()
        self.assertEqual(len(catalog.game("game").version("1.0").builds), 1)
        self.assertIn("unknown field 'lauch'", catalog.problems[0])

    def test_clashing_versions_are_rejected(self):
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", GOOD_SOURCE), version_with("1.0", GOOD_SOURCE)]))
        catalog = self.load()
        self.assertEqual(len(catalog.game("game").versions), 1)
        self.assertIn("clashes", catalog.problems[0])

    def test_launch_path_must_stay_inside_the_game_folder(self):
        for bad in ("../evil.exe", "/bin/sh", "..\\evil.exe"):
            with self.subTest(path=bad):
                write_game(self.catalog_dir, "game", game_with([version_with("1.0", GOOD_SOURCE, {"type": "executable", "path": bad})]))
                catalog = self.load()
                self.assertEqual(catalog.game("game").version("1.0").builds, [])
                self.assertIn("'path' must be relative", catalog.problems[0])

    def test_images_must_be_inside_the_game_folder(self):
        data = game_with([])
        data["icon"] = "../../secret.png"
        write_game(self.catalog_dir, "game", data)
        catalog = self.load()
        self.assertIsNone(catalog.game("game").icon)
        self.assertIn("'icon' must be an image file inside", catalog.problems[0])

    def test_bad_checksum_and_repo_are_rejected(self):
        write_game(self.catalog_dir, "game", game_with([
            version_with("1.0", {**GOOD_SOURCE, "sha256": "abc"}),
            version_with("1.1", {"type": "github-release", "repo": "not a repo", "tag": "v1", "asset": "a.zip"}),
        ]))
        catalog = self.load()
        self.assertTrue(all(not v.builds for v in catalog.game("game").versions))
        self.assertEqual(len(catalog.problems), 2)

    def test_platform_specific_build_wins_over_any(self):
        entry = version_with("1.0", GOOD_SOURCE, platform="any")
        entry["builds"].append({"platform": "windows", "source": GOOD_SOURCE,
                                "launch": {"type": "executable", "path": "Game.exe"}})
        write_game(self.catalog_dir, "game", game_with([entry]))
        version = self.load().game("game").version("1.0")
        self.assertEqual(version.build_for("windows").launch["path"], "Game.exe")
        self.assertEqual(version.build_for("linux").launch["path"], "main.py")

    def test_no_build_for_this_platform(self):
        write_game(self.catalog_dir, "game", game_with([version_with("1.0", GOOD_SOURCE, platform="macos")]))
        self.assertIsNone(self.load().game("game").version("1.0").build_for("windows"))


if __name__ == "__main__":
    unittest.main()

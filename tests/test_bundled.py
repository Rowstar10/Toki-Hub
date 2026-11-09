"""How the hub behaves as Toki Hub.exe: the built-in catalog plus the player's own catalog folder."""
import sys
import time
import unittest
from pathlib import Path
from unittest import mock

from tests.helpers import HubTestCase, game_with, version_with, write_game
from tokihub import app, paths
from tokihub.catalog import Catalog

SOURCE = {"type": "url", "url": "https://example.com/game.zip"}


class FrozenPathTests(unittest.TestCase):
    def test_exe_reads_its_built_in_catalog_then_the_players(self):
        with mock.patch.object(sys, "frozen", True, create=True), \
                mock.patch.object(sys, "_MEIPASS", "/unpacked", create=True):
            self.assertEqual(paths.ui_dir(), Path("/unpacked/ui"))
            self.assertEqual(paths.catalog_dirs(Path("/data")), [Path("/unpacked/catalog"), Path("/data/catalog")])
            self.assertEqual(paths.catalog_dir(Path("/data")), Path("/data/catalog"))

    def test_source_checkout_reads_the_repository_catalog(self):
        self.assertEqual(paths.catalog_dirs(Path("/data")), [paths.app_dir() / "catalog"])


class LayeredCatalogTests(HubTestCase):
    def setUp(self):
        super().setUp()
        self.built_in = self.tmp / "built-in"
        self.mine = self.tmp / "mine"
        write_game(self.built_in, "hu-taong", game_with([version_with("1.0", SOURCE)], title="Hu Taong"))
        write_game(self.built_in, "other", game_with([], title="Other"))

    def test_players_games_are_added_and_replace_built_in_ones(self):
        folder = write_game(self.mine, "hu-taong", {**game_with([version_with("1.1", SOURCE)], title="Hu Taong (mine)"),
                                                    "icon": "icon.png"})
        (folder / "icon.png").write_bytes(b"\x89PNG mine")
        write_game(self.mine, "new-game", game_with([], title="New Game"))
        catalog = Catalog.load([self.built_in, self.mine])
        self.assertEqual(catalog.problems, [])
        self.assertEqual([g.title for g in catalog.games.values()], ["Hu Taong (mine)", "New Game", "Other"])
        self.assertEqual(catalog.game("hu-taong").folder, folder)

    def test_a_missing_players_folder_is_fine(self):
        catalog = Catalog.load([self.built_in, self.tmp / "not-made-yet"])
        self.assertEqual(catalog.problems, [])
        self.assertEqual(len(catalog.games), 2)

    def test_no_catalog_at_all_is_reported(self):
        self.assertIn("Catalog folder not found", Catalog.load([self.tmp / "a", self.tmp / "b"]).problems[0])

    def test_hub_adds_new_games_to_the_last_folder(self):
        from tokihub.hub import Hub
        hub = Hub(catalog_dir=[self.built_in, self.mine], data_dir=self.data_dir)
        self.assertEqual(hub.catalog_dir, self.mine)
        self.assertEqual(hub.state()["places"]["catalog"], str(self.mine))


class ClosedPageTests(unittest.TestCase):
    def test_quits_only_after_the_page_has_gone_quiet(self):
        server = mock.Mock(last_api_call=None)
        self.assertFalse(app._page_closed(server), "never quit before the page has loaded")
        server.last_api_call = time.monotonic() - 5
        self.assertFalse(app._page_closed(server))
        server.last_api_call = time.monotonic() - app.CLOSED_PAGE_TIMEOUT - 1
        self.assertTrue(app._page_closed(server))

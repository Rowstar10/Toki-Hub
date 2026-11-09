"""The hub's core: everything the UI can ask for goes through a method of `Hub`.

The UI calls these over HTTP (see server.py); only the names in `server.API_METHODS` are
reachable. Every method takes and returns plain JSON-friendly values.
"""
import shutil
import urllib.parse
from pathlib import Path

from tokihub import __version__, paths
from tokihub.catalog import Catalog
from tokihub.errors import HubError
from tokihub.installer import Installer
from tokihub.launcher import Launcher, open_with_system
from tokihub.settings import Settings
from tokihub.sources import GITHUB_API, SourceResolver


class Hub:
    def __init__(self, catalog_dir=None, data_dir=None, github_api=GITHUB_API, platform=None):
        self.catalog_dir = Path(catalog_dir or paths.catalog_dir())
        self.data_dir = Path(data_dir or paths.data_dir())
        self.platform = platform or paths.current_platform()
        self.settings = Settings(self.data_dir / "settings.json", self.data_dir)
        self.resolver = SourceResolver(lambda: self.settings.github_token, github_api)
        self.launcher = Launcher(self.settings, self.data_dir / "logs")
        self.installer = Installer(self.settings, self.resolver, self.data_dir / "downloads",
                                   self.launcher.is_running, self.platform)
        self.catalog = Catalog.load(self.catalog_dir)
        self.window = None      # the pywebview window, when the hub runs in one

    # -- Reading state --

    def state(self):
        """Everything the UI needs to draw itself from scratch."""
        return {
            "hub_version": __version__,
            "platform": self.platform,
            "games": [self._game_json(game) for game in self.catalog.games.values()],
            "problems": list(self.catalog.problems),
            "settings": self.settings.for_ui(),
            "places": {"catalog": str(self.catalog_dir), "data": str(self.data_dir)},
            "native_window": self.window is not None,
            **self.poll(),
        }

    def poll(self, after_event=0):
        """The parts of the state that change while the hub is open."""
        return {
            "installed": {game_id: {version: _installed_json(info) for version, info in versions.items()}
                          for game_id, versions in self.installer.installed().items()},
            "tasks": self.installer.tasks(),
            "running": self.launcher.running(),
            "events": self.launcher.events_since(after_event),
        }

    def reload(self):
        self.catalog = Catalog.load(self.catalog_dir)
        return self.state()

    # -- Games --

    def install(self, game_id, version):
        game = self.catalog.game(game_id)
        return self.installer.start(game, game.version(version))

    def cancel(self, task_id):
        return self.installer.cancel(task_id)

    def uninstall(self, game_id, version):
        self.installer.uninstall(game_id, version)

    def launch(self, game_id, version):
        game = self.catalog.game(game_id)
        installed = self.installer.installed_version(game_id, version)
        if not installed:
            raise HubError(f"{game.title} {version} is not installed yet.")
        return self.launcher.launch(game, version, installed)

    def stop(self, game_id, version):
        self.launcher.stop(game_id, version)

    def open_folder(self, game_id, version):
        installed = self.installer.installed_version(game_id, version)
        if not installed:
            raise HubError("That version is not installed.")
        open_with_system(installed["path"])

    def open_log(self, game_id, version):
        log = self.launcher.log_path(game_id, version)
        if not log.is_file():
            raise HubError("There is no log yet: launch the game first.")
        open_with_system(log)

    def open_place(self, which):
        places = {"catalog": self.catalog_dir, "data": self.data_dir, "games": self.settings.install_dir,
                  "logs": self.launcher.logs_dir}
        if which not in places:
            raise HubError(f"Unknown place '{which}'.")
        places[which].mkdir(parents=True, exist_ok=True)
        open_with_system(places[which])

    # -- Settings --

    def save_settings(self, changes):
        if not isinstance(changes, dict):
            raise HubError("Settings must be sent as an object.")
        clean = {}
        if "install_dir" in changes:
            folder = str(changes["install_dir"]).strip()
            if folder and not Path(folder).expanduser().is_absolute():
                raise HubError("The install folder must be a full path, like C:\\Games\\TokiHub.")
            clean["install_dir"] = folder
        if "python" in changes:
            python = str(changes["python"]).strip()
            if python and not (Path(python).is_file() or shutil.which(python)):
                raise HubError(f"Python interpreter not found: {python}")
            clean["python"] = python
        token = str(changes.get("github_token", "")).strip()
        if token:
            clean["github_token"] = token
        self.settings.update(clean)
        return self.settings.for_ui()

    def clear_token(self):
        self.settings.update({"github_token": ""})
        return self.settings.for_ui()

    def pick_folder(self):
        """Show a folder picker. Only possible in the hub's own window, not in a browser tab."""
        if self.window is None:
            return None
        import webview
        dialog = getattr(getattr(webview, "FileDialog", None), "FOLDER", None)
        if dialog is None:
            dialog = webview.FOLDER_DIALOG
        chosen = self.window.create_file_dialog(dialog)
        return str(chosen[0]) if chosen else None

    # -- Helpers --

    def _game_json(self, game):
        return {
            "id": game.id,
            "title": game.title,
            "tagline": game.tagline,
            "description": game.description,
            "developer": game.developer,
            "tags": game.tags,
            "accent": game.accent,
            "icon": _image_url(game, game.icon),
            "cover": _image_url(game, game.cover),
            "versions": [{
                "version": v.version,
                "date": v.date,
                "notes": v.notes,
                "platforms": sorted({b.platform for b in v.builds}),
                "build": _build_json(v.build_for(self.platform)),
            } for v in game.versions],
        }


def _build_json(build):
    if build is None:
        return None
    return {"platform": build.platform, "source": build.source_label(), "launch": build.launch["type"],
            "note": build.note}


def _installed_json(info):
    return {"path": info.get("path", ""), "installed_at": info.get("installed_at", ""), "size": info.get("size", 0)}


def _image_url(game, path):
    if path is None:
        return None
    relative = path.relative_to(game.folder).as_posix()
    return f"/catalog/{game.id}/{urllib.parse.quote(relative)}"

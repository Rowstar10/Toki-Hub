"""Where Toki Hub keeps things.

- The *bundle folder* holds `ui/` and the shipped `catalog/`. Running from source it is the
  repository root; in Toki Hub.exe it is the folder PyInstaller unpacks the exe into.
- The *data folder* holds settings, installed games, downloads and logs. It is per user, so
  updating or moving the hub never touches installed games. TOKIHUB_DATA_DIR overrides it.

Catalogs: from source, the hub reads the repository's `catalog/`. Toki Hub.exe reads the catalog
built into it and then `<data folder>/catalog`, where games can be added without rebuilding the
exe (a game there replaces a built-in one with the same id).
"""
import os
import re
import sys
from pathlib import Path

APP_NAME = "Toki Hub"


def is_frozen():
    return bool(getattr(sys, "frozen", False))


def app_dir():
    if is_frozen():
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent.parent


def bundle_dir():
    if is_frozen():
        return Path(getattr(sys, "_MEIPASS", app_dir()))
    return app_dir()


def catalog_dirs(data=None):
    """The catalog folders to read, in order; later ones override earlier ones."""
    if is_frozen():
        return [bundle_dir() / "catalog", Path(data or data_dir()) / "catalog"]
    return [app_dir() / "catalog"]


def catalog_dir(data=None):
    """The catalog folder people edit: where new games go and what "Open catalog folder" opens."""
    return catalog_dirs(data)[-1]


def ui_dir():
    return bundle_dir() / "ui"


def data_dir():
    override = os.environ.get("TOKIHUB_DATA_DIR")
    if override:
        return Path(override).expanduser().resolve()
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA") or os.environ.get("APPDATA") or Path.home() / "AppData" / "Local"
        return Path(base) / APP_NAME
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / APP_NAME
    base = os.environ.get("XDG_DATA_HOME") or Path.home() / ".local" / "share"
    return Path(base) / APP_NAME


def current_platform():
    """The platform name used by `builds[].platform` in the catalog."""
    if sys.platform == "win32":
        return "windows"
    if sys.platform == "darwin":
        return "macos"
    return "linux"


def safe_name(text):
    """A string that is safe to use as a single folder or file name on every OS."""
    name = re.sub(r"[^A-Za-z0-9._-]+", "_", text).strip("._")
    return name or "_"


def is_within(path, folder):
    """True when `path` is `folder` or inside it, after resolving `..` and symlinks."""
    try:
        Path(path).resolve().relative_to(Path(folder).resolve())
        return True
    except ValueError:
        return False

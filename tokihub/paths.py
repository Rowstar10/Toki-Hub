"""Where Toki Hub keeps things.

- The *app folder* holds `catalog/` (the list of games). Running from source it is the repository
  root; in a PyInstaller build it is the folder next to the .exe, so the catalog stays editable.
- The *bundle folder* holds `ui/`. Running from source it is the same as the app folder.
- The *data folder* holds settings, installed games, downloads and logs. It is per user, so
  updating or moving the hub never touches installed games. TOKIHUB_DATA_DIR overrides it.
"""
import os
import re
import sys
from pathlib import Path

APP_NAME = "TokiHub"


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


def catalog_dir():
    return app_dir() / "catalog"


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

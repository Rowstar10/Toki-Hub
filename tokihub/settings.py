"""Hub settings, stored as settings.json in the data folder."""
import json
import os
import sys
import threading
from pathlib import Path

DEFAULTS = {
    "install_dir": "",     # empty: <data folder>/games
    "python": "",          # empty: detect (used for games that run from Python source)
    "github_token": "",    # for downloading from private GitHub repositories
}


class Settings:
    def __init__(self, path, data_dir):
        self.path = Path(path)
        self.data_dir = Path(data_dir)
        self._lock = threading.Lock()
        self._values = dict(DEFAULTS)
        try:
            saved = json.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            saved = {}
        if isinstance(saved, dict):
            for key, default in DEFAULTS.items():
                if isinstance(saved.get(key), type(default)):
                    self._values[key] = saved[key]

    def get(self, key):
        with self._lock:
            return self._values[key]

    def update(self, changes):
        """Apply {key: value} changes (unknown keys and wrong types are ignored) and save."""
        with self._lock:
            for key, value in changes.items():
                if key in DEFAULTS and isinstance(value, type(DEFAULTS[key])):
                    self._values[key] = value.strip() if isinstance(value, str) else value
            self.path.parent.mkdir(parents=True, exist_ok=True)
            tmp = self.path.with_suffix(".tmp")
            tmp.write_text(json.dumps(self._values, indent=2), encoding="utf-8")
            tmp.replace(self.path)

    # -- Derived values --

    @property
    def install_dir(self):
        custom = self.get("install_dir")
        return Path(custom).expanduser() if custom else self.data_dir / "games"

    @property
    def github_token(self):
        return self.get("github_token") or os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or ""

    def python_command(self):
        """The interpreter for `"launch": {"type": "python"}` games, as a list for subprocess."""
        custom = self.get("python")
        if custom:
            return [custom]
        return default_python()

    def for_ui(self):
        """Settings as the UI sees them. The token itself never leaves the hub."""
        return {
            "install_dir": self.get("install_dir"),
            "install_dir_effective": str(self.install_dir),
            "python": self.get("python"),
            "python_effective": " ".join(self.python_command()),
            "has_token": bool(self.get("github_token")),
            "token_from_env": not self.get("github_token") and bool(self.github_token),
        }


def default_python():
    import shutil
    if not getattr(sys, "frozen", False) and sys.executable:
        return [sys.executable]
    if sys.platform == "win32" and shutil.which("py"):
        return ["py", "-3"]
    for name in ("python3", "python"):
        found = shutil.which(name)
        if found:
            return [found]
    return ["python"]

"""The game catalog: one folder per game under `catalog/`, each with a `game.json`.

See catalog/README.md for the format. Loading never raises for a bad entry: whatever can be
used is kept, and everything wrong is collected in `Catalog.problems` so the hub can show it
and `python main.py check` can list it.
"""
import json
import re
from dataclasses import dataclass, field
from pathlib import Path

from tokihub.errors import HubError
from tokihub.paths import is_within, safe_name
from tokihub.versions import newest_first

PLATFORMS = ("windows", "macos", "linux", "any")

# For each source type: (required fields, optional fields)
SOURCE_FIELDS = {
    "url": (("url",), ("sha256", "filename")),
    "github-release": (("repo", "tag", "asset"), ("sha256",)),
    "github-archive": (("repo", "ref"), ()),
    "local": (("path",), ("sha256",)),
}
LAUNCH_TYPES = ("executable", "python", "open")

GAME_FIELDS = {"title", "tagline", "description", "developer", "tags", "icon", "cover", "accent", "versions"}
VERSION_FIELDS = {"version", "date", "notes", "builds"}
BUILD_FIELDS = {"platform", "source", "launch", "note"}
# "python" and "packages" say what a python game needs. The Tauri version installs them; this version
# checks them but uses the Python set in Settings.
LAUNCH_FIELDS = {"type", "path", "args", "python", "packages"}

GAME_ID = re.compile(r"^[a-z0-9][a-z0-9-]*$")
REPO = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
COLOUR = re.compile(r"^#[0-9a-fA-F]{6}$")
SHA256 = re.compile(r"^[0-9a-fA-F]{64}$")
IMAGE_TYPES = (".png", ".jpg", ".jpeg", ".webp", ".gif", ".svg")


@dataclass
class Build:
    platform: str
    source: dict
    launch: dict
    note: str = ""

    def source_label(self):
        s = self.source
        kind = s["type"]
        if kind == "github-release":
            return f"GitHub release {s['repo']} @ {s['tag']}"
        if kind == "github-archive":
            ref = s["ref"]
            return f"GitHub source {s['repo']} @ {ref[:10] if len(ref) == 40 else ref}"
        if kind == "local":
            return f"Local files: {s['path']}"
        return s["url"]


@dataclass
class Version:
    version: str
    date: str = ""
    notes: str = ""
    builds: list = field(default_factory=list)

    @property
    def folder(self):
        return safe_name(self.version)

    def build_for(self, platform):
        """The build to use on `platform`: an exact match first, otherwise an "any" build."""
        for wanted in (platform, "any"):
            for build in self.builds:
                if build.platform == wanted:
                    return build
        return None


@dataclass
class Game:
    id: str
    folder: Path
    title: str
    tagline: str = ""
    description: str = ""
    developer: str = ""
    tags: list = field(default_factory=list)
    accent: str = ""
    icon: Path = None
    cover: Path = None
    versions: list = field(default_factory=list)   # newest first

    def version(self, name):
        for v in self.versions:
            if v.version == name:
                return v
        raise HubError(f"{self.title} has no version '{name}'.")


class Catalog:
    def __init__(self, folders, games, problems):
        self.folders = [Path(f) for f in folders]
        self.games = games          # id -> Game, ordered by title
        self.problems = problems    # list of str

    @classmethod
    def load(cls, folders):
        """Read one catalog folder, or a list of them. A game in a later folder replaces one with
        the same id in an earlier folder. Folders that do not exist are skipped."""
        folders = [Path(folders)] if isinstance(folders, (str, Path)) else [Path(f) for f in folders]
        games, problems = {}, []
        existing = [f for f in folders if f.is_dir()]
        if not existing:
            return cls(folders, {}, [f"Catalog folder not found: {folders[-1]}"])
        for folder in existing:
            for game_dir in sorted(p for p in folder.iterdir() if p.is_dir() and not p.name.startswith((".", "_"))):
                game = _load_game(game_dir, problems)
                if game:
                    games[game.id] = game
        ordered = dict(sorted(games.items(), key=lambda item: item[1].title.lower()))
        return cls(folders, ordered, problems)

    def game(self, game_id):
        try:
            return self.games[game_id]
        except KeyError:
            raise HubError(f"There is no game '{game_id}' in the catalog.") from None


class _Checker:
    """Collects problems, each prefixed with where in game.json it was found."""

    def __init__(self, prefix, problems):
        self.prefix = prefix
        self.problems = problems

    def report(self, where, message):
        self.problems.append(f"{self.prefix}: {where + ': ' if where else ''}{message}")

    def text(self, obj, key, where, required=False):
        value = obj.get(key)
        if value is None:
            if required:
                self.report(where, f"'{key}' is required")
            return ""
        if not isinstance(value, str) or (required and not value.strip()):
            self.report(where, f"'{key}' must be {'non-empty ' if required else ''}text")
            return ""
        return value.strip()

    def unknown(self, obj, allowed, where):
        for key in obj:
            if key not in allowed and not key.startswith("_"):
                self.report(where, f"unknown field '{key}' (ignored; a typo?)")


def _load_game(game_dir, problems):
    check = _Checker(f"{game_dir.name}/game.json", problems)
    path = game_dir / "game.json"
    if not GAME_ID.match(game_dir.name):
        check.report("", "folder name must be lowercase letters, digits and dashes (it is the game's id)")
        return None
    if not path.is_file():
        check.report("", "file is missing")
        return None
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError) as e:
        check.report("", f"cannot be read: {e}")
        return None
    except json.JSONDecodeError as e:
        check.report("", f"is not valid JSON (line {e.lineno}, column {e.colno}: {e.msg})")
        return None
    if not isinstance(data, dict):
        check.report("", "must contain a JSON object { ... }")
        return None

    check.unknown(data, GAME_FIELDS, "")
    title = check.text(data, "title", "", required=True)
    if not title:
        return None

    tags = data.get("tags", [])
    if not isinstance(tags, list) or not all(isinstance(t, str) for t in tags):
        check.report("", "'tags' must be a list of text")
        tags = []

    accent = check.text(data, "accent", "")
    if accent and not COLOUR.match(accent):
        check.report("", "'accent' must be a colour like \"#3fb0ef\"")
        accent = ""

    game = Game(id=game_dir.name, folder=game_dir, title=title,
                tagline=check.text(data, "tagline", ""),
                description=check.text(data, "description", ""),
                developer=check.text(data, "developer", ""),
                tags=[t.strip() for t in tags if t.strip()], accent=accent,
                icon=_image(data, "icon", game_dir, check),
                cover=_image(data, "cover", game_dir, check))

    versions = data.get("versions", [])
    if not isinstance(versions, list):
        check.report("", "'versions' must be a list")
        versions = []
    seen = {}
    for i, entry in enumerate(versions):
        version = _load_version(entry, f"versions[{i}]", check)
        if not version:
            continue
        if version.folder in seen:
            check.report(f"versions[{i}]", f"version '{version.version}' clashes with '{seen[version.folder]}'")
            continue
        seen[version.folder] = version.version
        game.versions.append(version)
    game.versions = newest_first(game.versions, key=lambda v: v.version)
    return game


def _image(data, key, game_dir, check):
    name = check.text(data, key, "")
    if not name:
        return None
    path = game_dir / name
    if not is_within(path, game_dir) or path.suffix.lower() not in IMAGE_TYPES:
        check.report("", f"'{key}' must be an image file inside the game's folder")
        return None
    if not path.is_file():
        check.report("", f"'{key}' file not found: {name}")
        return None
    return path


def _load_version(entry, where, check):
    if not isinstance(entry, dict):
        check.report(where, "must be an object { ... }")
        return None
    check.unknown(entry, VERSION_FIELDS, where)
    name = check.text(entry, "version", where, required=True)
    if not name:
        return None
    if len(name) > 64:
        check.report(where, "'version' is too long (64 characters at most)")
        return None
    where = f"version {name}"
    version = Version(version=name, date=check.text(entry, "date", where),
                      notes=check.text(entry, "notes", where))
    builds = entry.get("builds", [])
    if not isinstance(builds, list):
        check.report(where, "'builds' must be a list")
        builds = []
    for i, build in enumerate(builds):
        loaded = _load_build(build, f"{where}: builds[{i}]", check)
        if loaded:
            version.builds.append(loaded)
    return version


def _load_build(entry, where, check):
    if not isinstance(entry, dict):
        check.report(where, "must be an object { ... }")
        return None
    check.unknown(entry, BUILD_FIELDS, where)
    platform = entry.get("platform", "any")
    if platform not in PLATFORMS:
        check.report(where, f"'platform' must be one of {', '.join(PLATFORMS)}")
        return None
    source = _load_source(entry.get("source"), where, check)
    launch = _load_launch(entry.get("launch"), where, check)
    note = check.text(entry, "note", where)
    if source is None or launch is None:
        return None
    return Build(platform=platform, source=source, launch=launch, note=note)


def _load_source(source, where, check):
    where = f"{where}: source"
    if not isinstance(source, dict):
        check.report(where, "is required and must be an object { \"type\": ... }")
        return None
    kind = source.get("type")
    if kind not in SOURCE_FIELDS:
        check.report(where, f"'type' must be one of {', '.join(SOURCE_FIELDS)}")
        return None
    required, optional = SOURCE_FIELDS[kind]
    check.unknown(source, {"type", *required, *optional}, where)
    result = {"type": kind}
    for key in required:
        value = check.text(source, key, where, required=True)
        if not value:
            return None
        result[key] = value
    for key in optional:
        value = check.text(source, key, where)
        if value:
            result[key] = value
    if "repo" in result and not REPO.match(result["repo"]):
        check.report(where, "'repo' must look like \"Owner/RepoName\"")
        return None
    if kind == "url" and not result["url"].startswith(("https://", "http://")):
        check.report(where, "'url' must start with https:// or http://")
        return None
    if "sha256" in result:
        if not SHA256.match(result["sha256"]):
            check.report(where, "'sha256' must be 64 hexadecimal characters")
            return None
        result["sha256"] = result["sha256"].lower()
    if "filename" in result and safe_name(result["filename"]) != result["filename"]:
        check.report(where, "'filename' may only use letters, digits, '.', '_' and '-'")
        return None
    return result


def _load_launch(launch, where, check):
    where = f"{where}: launch"
    if not isinstance(launch, dict):
        check.report(where, "is required and must be an object { \"type\": ..., \"path\": ... }")
        return None
    check.unknown(launch, LAUNCH_FIELDS, where)
    kind = launch.get("type")
    if kind not in LAUNCH_TYPES:
        check.report(where, f"'type' must be one of {', '.join(LAUNCH_TYPES)}")
        return None
    path = check.text(launch, "path", where, required=True)
    if not path:
        return None
    unix_style = path.replace("\\", "/")
    if unix_style.startswith("/") or re.match(r"^[A-Za-z]:", path) or ".." in Path(unix_style).parts:
        check.report(where, "'path' must be relative to the installed game's folder")
        return None
    args = launch.get("args", [])
    if not isinstance(args, list) or not all(isinstance(a, str) for a in args):
        check.report(where, "'args' must be a list of text")
        return None
    result = {"type": kind, "path": path.replace("\\", "/"), "args": args}
    python = check.text(launch, "python", where)
    if python and not re.match(r"^\d+(\.\d+){0,2}$", python):
        check.report(where, "'python' must be a version like \"3.12\"")
        return None
    packages = launch.get("packages", [])
    if not isinstance(packages, list) or not all(isinstance(p, str) and p.strip() and not p.strip().startswith("-")
                                                 for p in packages):
        check.report(where, "'packages' must be a list of package names, like [\"pygame==2.6.1\"]")
        return None
    if kind != "python" and (python or packages):
        check.report(where, "'python' and 'packages' only apply to python games (ignored)")
    elif python or packages:
        result.update(python=python, packages=[p.strip() for p in packages])
    return result

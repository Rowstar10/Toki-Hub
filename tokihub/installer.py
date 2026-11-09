"""Downloading and installing game versions.

Each version installs into its own folder, <install dir>/<game id>/<version>/, so any number of
versions sit side by side. Installs run in a background thread and are atomic: files are
unpacked into a staging folder first and only moved into place once everything succeeded, so a
failed or cancelled download never leaves a half-installed version behind.

An installed version is recognised by the `.tokihub.json` marker in its folder, which also
records how to launch it.
"""
import hashlib
import itertools
import json
import logging
import os
import shutil
import stat
import sys
import threading
import time
import urllib.error
import urllib.request
import zipfile
from datetime import datetime, timezone
from pathlib import Path

from tokihub.errors import HubError
from tokihub.paths import current_platform, is_within, safe_name
from tokihub.sources import USER_AGENT, Download, github_error

log = logging.getLogger(__name__)

MARKER = ".tokihub.json"
CHUNK = 256 * 1024
STALE_AFTER = 3600      # seconds before leftovers of an interrupted install are swept away
ACTIVE = ("queued", "downloading", "installing")


class Cancelled(Exception):
    pass


class Task:
    _ids = itertools.count(1)

    def __init__(self, game, version):
        self.id = f"t{next(self._ids)}"
        self.game_id = game.id
        self.title = game.title
        self.version = version.version
        self.status = "queued"
        self.received = 0
        self.total = 0
        self.error = ""
        self.cancel_requested = False

    @property
    def active(self):
        return self.status in ACTIVE

    def to_dict(self):
        return {"id": self.id, "game_id": self.game_id, "title": self.title, "version": self.version,
                "status": self.status, "received": self.received, "total": self.total, "error": self.error}


class Installer:
    def __init__(self, settings, resolver, downloads_dir, is_running=lambda game_id, version: False,
                 platform=None):
        self.settings = settings
        self.resolver = resolver
        self.downloads_dir = Path(downloads_dir)
        self.is_running = is_running
        self.platform = platform or current_platform()
        self._tasks = {}            # (game_id, version) -> latest Task for it
        self._lock = threading.Lock()
        self._sweep_leftovers()

    # -- Queries --

    def version_dir(self, game_id, version_name):
        return self.settings.install_dir / game_id / safe_name(version_name)

    def installed(self):
        """{game_id: {version: marker info}} for everything in the install folder."""
        found = {}
        root = self.settings.install_dir
        if not root.is_dir():
            return found
        for marker in root.glob(f"*/*/{MARKER}"):
            if marker.parent.name.startswith("."):     # an install still in progress
                continue
            try:
                info = json.loads(marker.read_text(encoding="utf-8"))
                game_id, version = info["game"], info["version"]
            except (OSError, ValueError, KeyError, TypeError):
                continue
            info["path"] = str(marker.parent)
            found.setdefault(game_id, {})[version] = info
        return found

    def installed_version(self, game_id, version_name):
        marker = self.version_dir(game_id, version_name) / MARKER
        try:
            info = json.loads(marker.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return None
        info["path"] = str(marker.parent)
        return info

    def tasks(self):
        with self._lock:
            return [task.to_dict() for task in self._tasks.values()]

    # -- Actions --

    def start(self, game, version):
        build = version.build_for(self.platform)
        if build is None:
            raise HubError(f"{game.title} {version.version} has no build for {self.platform}.")
        if self.is_running(game.id, version.version):
            raise HubError(f"Close {game.title} {version.version} before reinstalling it.")
        key = (game.id, version.version)
        with self._lock:
            current = self._tasks.get(key)
            if current and current.active:
                raise HubError(f"{game.title} {version.version} is already downloading.")
            task = Task(game, version)
            self._tasks[key] = task
        threading.Thread(target=self._run, args=(task, game, version, build),
                         name=f"install-{task.id}", daemon=True).start()
        return task.to_dict()

    def cancel(self, task_id):
        with self._lock:
            for task in self._tasks.values():
                if task.id == task_id and task.active:
                    task.cancel_requested = True
                    return True
        return False

    def uninstall(self, game_id, version_name):
        folder = self.version_dir(game_id, version_name)
        if not (folder / MARKER).is_file():
            raise HubError(f"Version {version_name} is not installed.")
        if self.is_running(game_id, version_name):
            raise HubError("Close the game before uninstalling it.")
        try:
            remove_tree(folder)
        except OSError as e:
            raise HubError(f"Could not delete {folder}: {e.strerror or e}") from None
        try:
            folder.parent.rmdir()       # tidy up the game's folder once its last version is gone
        except OSError:
            pass

    def _sweep_leftovers(self):
        """Remove what an install interrupted by a crash or power cut left behind."""
        cutoff = time.time() - STALE_AFTER
        leftovers = list(self.settings.install_dir.glob("*/.staging-*")) + list(self.downloads_dir.glob("*.part"))
        for path in leftovers:
            try:
                if path.stat().st_mtime < cutoff:
                    remove_tree(path, ignore_errors=True) if path.is_dir() else path.unlink()
            except OSError:
                pass

    # -- The install itself (runs in a background thread) --

    def _run(self, task, game, version, build):
        target = self.version_dir(game.id, version.version)
        staging = target.parent / f".staging-{safe_name(version.version)}-{task.id}"
        temp_file = None
        try:
            plan = self.resolver.resolve(build.source, game.folder)
            if isinstance(plan, Download):
                task.status = "downloading"
                temp_file = self._download(plan, task)
                task.status = "installing"
                self._unpack(temp_file, plan.filename, staging, task)
            else:
                task.status = "installing"
                self._copy_local(plan, staging, task)
            self._check_cancel(task)
            if not is_within(staging / build.launch["path"], staging) or not (staging / build.launch["path"]).exists():
                raise HubError(f"The downloaded files do not contain '{build.launch['path']}', which the catalog "
                               "says to launch. Check `launch.path` for this build.")
            write_marker(staging, game, version, build)
            if self.is_running(game.id, version.version):
                raise HubError(f"Close {game.title} {version.version} so the new files can be put in place.")
            if target.exists():
                remove_tree(target)
            move_into_place(staging, target)
            task.status = "done"
        except Cancelled:
            task.status = "cancelled"
        except HubError as e:
            task.status, task.error = "error", str(e)
        except OSError as e:
            task.status, task.error = "error", f"File error: {e.strerror or e} ({e.filename or ''})".replace(" ()", "")
        except Exception as e:      # keep the hub alive whatever happens in a download
            log.exception("install %s %s failed", game.id, version.version)
            task.status, task.error = "error", f"Unexpected error: {e}"
        finally:
            if staging.exists():
                remove_tree(staging, ignore_errors=True)
            if temp_file is not None:
                try:
                    temp_file.unlink()
                except OSError:
                    pass

    @staticmethod
    def _check_cancel(task):
        if task.cancel_requested:
            raise Cancelled()

    def _download(self, plan, task):
        self.downloads_dir.mkdir(parents=True, exist_ok=True)
        part = self.downloads_dir / f"{task.id}-{plan.filename}.part"
        request = urllib.request.Request(plan.url, headers={"User-Agent": USER_AGENT, **plan.headers})
        for key, value in plan.auth_headers.items():
            request.add_unredirected_header(key, value)   # never sent on to a redirect's host
        try:
            response = urllib.request.urlopen(request, timeout=30)
        except urllib.error.HTTPError as e:
            if plan.github_what:
                raise github_error(e, plan.github_what) from None
            raise HubError(f"Download failed: HTTP {e.code} {e.reason} ({plan.url})") from None
        except (urllib.error.URLError, OSError) as e:
            raise HubError(f"Could not connect to download {plan.filename}: {getattr(e, 'reason', e)}") from None
        digest = hashlib.sha256()
        with response, open(part, "wb") as out:
            try:
                task.total = int(response.headers.get("Content-Length") or 0)
            except ValueError:
                task.total = 0
            while True:
                self._check_cancel(task)
                try:
                    chunk = response.read(CHUNK)
                except OSError as e:
                    raise HubError(f"The download was interrupted: {e}") from None
                if not chunk:
                    break
                out.write(chunk)
                digest.update(chunk)
                task.received += len(chunk)
        if task.total and task.received < task.total:
            raise HubError("The download ended early. Please try again.")
        if plan.sha256 and digest.hexdigest() != plan.sha256:
            raise HubError(f"{plan.filename} does not match the sha256 in the catalog, so it was not installed. "
                           "The file may be damaged, or the catalog entry needs updating.")
        return part

    def _unpack(self, file, filename, staging, task):
        staging.mkdir(parents=True)
        if filename.lower().endswith(".zip"):
            extract_zip(file, staging, lambda: self._check_cancel(task))
        else:
            dest = staging / filename
            shutil.copyfile(file, dest)
            make_executable(dest)

    def _copy_local(self, plan, staging, task):
        source = plan.path
        if plan.sha256 and source.is_file() and file_sha256(source) != plan.sha256:
            raise HubError(f"{source.name} does not match the sha256 in the catalog, so it was not installed.")
        if source.is_dir():
            shutil.copytree(source, staging, ignore=shutil.ignore_patterns(".git", "__pycache__", MARKER))
        elif source.suffix.lower() == ".zip":
            staging.mkdir(parents=True)
            extract_zip(source, staging, lambda: self._check_cancel(task))
        else:
            staging.mkdir(parents=True)
            shutil.copy2(source, staging / source.name)


def extract_zip(zip_path, dest, check_cancel=lambda: None):
    """Extract a zip into `dest`. A single top-level folder (as in GitHub source zips) is dropped,
    so the game's files land directly in `dest`. Entries that would escape `dest` are refused."""
    try:
        archive = zipfile.ZipFile(zip_path)
    except zipfile.BadZipFile:
        raise HubError("The downloaded file is not a valid zip archive.") from None
    with archive:
        entries = [(info, info.filename.replace("\\", "/")) for info in archive.infolist() if not info.is_dir()]
        if not entries:
            raise HubError("The zip archive is empty.")
        tops = {name.split("/", 1)[0] for _, name in entries}
        strip = len(tops) == 1 and all("/" in name for _, name in entries)
        cut = len(next(iter(tops))) + 1 if strip else 0
        for info, name in entries:
            check_cancel()
            relative = name[cut:]
            target = dest / relative
            if not relative or relative.startswith("/") or ":" in relative or not is_within(target, dest):
                raise HubError(f"The zip archive contains an unsafe path ({name}), so it was not installed.")
            target.parent.mkdir(parents=True, exist_ok=True)
            try:
                with archive.open(info) as src, open(target, "wb") as out:
                    shutil.copyfileobj(src, out, CHUNK)
            except (zipfile.BadZipFile, EOFError, zipfile.LargeZipFile) as e:
                raise HubError(f"The zip archive is damaged ({name}: {e}).") from None
            mode = (info.external_attr >> 16) & 0o777
            if mode & 0o111 and os.name != "nt":
                os.chmod(target, mode | 0o600)


def move_into_place(staging, target):
    """Rename the finished staging folder to its final name. On Windows, antivirus scanners often hold
    freshly written files open for a moment, so a failed rename is retried briefly."""
    for attempt in range(6):
        try:
            os.replace(staging, target)
            return
        except PermissionError:
            if attempt == 5:
                raise
            time.sleep(0.5)


def write_marker(folder, game, version, build):
    size = sum(f.stat().st_size for f in Path(folder).rglob("*") if f.is_file())
    marker = {
        "game": game.id,
        "version": version.version,
        "platform": build.platform,
        "source": build.source,
        "launch": build.launch,
        "installed_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "size": size,
    }
    (Path(folder) / MARKER).write_text(json.dumps(marker, indent=2), encoding="utf-8")


def make_executable(path):
    if os.name != "nt":
        mode = os.stat(path).st_mode
        os.chmod(path, mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(CHUNK), b""):
            digest.update(chunk)
    return digest.hexdigest()


def remove_tree(path, ignore_errors=False):
    """shutil.rmtree that also removes read-only files (common on Windows), retrying briefly
    in case a just-closed game still holds a file open."""
    def make_writable_and_retry(func, target, _exc):
        os.chmod(target, stat.S_IWRITE)
        func(target)

    for attempt in range(3):
        try:
            if sys.version_info >= (3, 12):
                shutil.rmtree(path, onexc=make_writable_and_retry)
            else:
                shutil.rmtree(path, onerror=make_writable_and_retry)
            return
        except FileNotFoundError:
            return
        except OSError:
            if attempt == 2:
                if ignore_errors:
                    return
                raise
            time.sleep(0.5)

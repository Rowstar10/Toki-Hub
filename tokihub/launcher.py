"""Starting installed games and keeping an eye on them.

Each run's output goes to <data folder>/logs/<game>-<version>.log. When a game exits, an
"exited" event is recorded; if it crashed, the event carries the end of the log so the hub can
say what went wrong (for example a missing Python package).
"""
import os
import subprocess
import sys
import threading
import time
from collections import deque
from datetime import datetime
from pathlib import Path

from tokihub.errors import HubError
from tokihub.installer import make_executable
from tokihub.paths import is_within, safe_name

LOG_TAIL_LINES = 12


class Launcher:
    def __init__(self, settings, logs_dir):
        self.settings = settings
        self.logs_dir = Path(logs_dir)
        self._running = {}                  # (game_id, version) -> dict with "process"
        self._events = deque(maxlen=50)
        self._next_event = 1
        self._lock = threading.Lock()

    # -- Queries --

    def is_running(self, game_id, version):
        with self._lock:
            return (game_id, version) in self._running

    def running(self):
        with self._lock:
            return [{"game_id": g, "version": v, "pid": info["process"].pid, "started": info["started"]}
                    for (g, v), info in self._running.items()]

    def events_since(self, after_id):
        with self._lock:
            return [event for event in self._events if event["id"] > after_id]

    def log_path(self, game_id, version):
        return self.logs_dir / f"{game_id}-{safe_name(version)}.log"

    # -- Actions --

    def launch(self, game, version, installed):
        """Start an installed version. `installed` is its marker info from the installer."""
        folder = Path(installed["path"])
        launch = installed["launch"]
        target = folder / launch["path"]
        if not is_within(target, folder):
            raise HubError("This version's launch path points outside its folder.")
        if not target.is_file():
            raise HubError(f"'{launch['path']}' is missing from the installed files. Try reinstalling "
                           f"{game.title} {version}.")
        if launch["type"] == "open":
            open_with_system(target)
            return {"tracked": False}

        if launch["type"] == "python":
            command = self.settings.python_command() + [str(target), *launch.get("args", [])]
        else:
            make_executable(target)
            command = [str(target), *launch.get("args", [])]

        key = (game.id, version)
        with self._lock:
            if key in self._running:
                raise HubError(f"{game.title} {version} is already running.")
            self.logs_dir.mkdir(parents=True, exist_ok=True)
            log_path = self.log_path(game.id, version)
            log_file = open(log_path, "w", encoding="utf-8", errors="replace")
            log_file.write(f"# {game.title} {version}, started {datetime.now():%Y-%m-%d %H:%M:%S}\n"
                           f"# {subprocess.list2cmdline(command)}\n# in {target.parent}\n\n")
            log_file.flush()
            try:
                process = subprocess.Popen(command, cwd=target.parent, stdin=subprocess.DEVNULL,
                                           stdout=log_file, stderr=subprocess.STDOUT,
                                           env=_environment(launch["type"]), **_detach_options(launch["type"]))
            except FileNotFoundError:
                log_file.close()
                if launch["type"] == "python":
                    raise HubError(f"Python was not found ('{command[0]}'). Set the Python interpreter in "
                                   "Settings.") from None
                raise HubError(f"Could not find {target.name} to start it.") from None
            except OSError as e:
                log_file.close()
                raise HubError(f"Could not start {game.title}: {e.strerror or e}") from None
            self._running[key] = {"process": process, "started": time.time(), "title": game.title}
        threading.Thread(target=self._watch, args=(key, process, log_file, log_path),
                         name=f"watch-{game.id}", daemon=True).start()
        return {"tracked": True, "pid": process.pid}

    def stop(self, game_id, version):
        with self._lock:
            info = self._running.get((game_id, version))
        if not info:
            raise HubError("That game is not running.")
        info["stopping"] = True
        info["process"].terminate()

    def _watch(self, key, process, log_file, log_path):
        code = process.wait()
        log_file.close()
        with self._lock:
            info = self._running.pop(key, {})
            stopped = info.get("stopping", False)
            event = {"id": self._next_event, "type": "exited", "game_id": key[0], "version": key[1],
                     "title": info.get("title", key[0]), "code": code, "stopped": stopped,
                     "seconds": round(time.time() - info.get("started", time.time()), 1),
                     "log_tail": read_tail(log_path) if code not in (0, None) and not stopped else ""}
            self._next_event += 1
            self._events.append(event)


def read_tail(path, lines=LOG_TAIL_LINES):
    try:
        with open(path, "rb") as f:
            f.seek(0, os.SEEK_END)
            f.seek(max(0, f.tell() - 8192))
            text = f.read().decode("utf-8", errors="replace")
    except OSError:
        return ""
    body = [line for line in text.splitlines() if not line.startswith("# ")]
    return "\n".join(body[-lines:]).strip()


def open_with_system(path):
    """Open a file or folder the way double-clicking it would."""
    path = str(path)
    try:
        if sys.platform == "win32":
            os.startfile(path)      # noqa: S606 - opening the player's own files is the point
        elif sys.platform == "darwin":
            subprocess.Popen(["open", path])
        else:
            subprocess.Popen(["xdg-open", path], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError as e:
        raise HubError(f"Could not open {path}: {e.strerror or e}") from None


def _environment(launch_type):
    env = dict(os.environ)
    if launch_type == "python":
        env["PYTHONUNBUFFERED"] = "1"       # so the log is complete if the game crashes
        env["PYTHONIOENCODING"] = "utf-8"
    return env


def _detach_options(launch_type):
    """Games keep running if the hub closes, and Python games do not open a console window."""
    if sys.platform == "win32":
        flags = subprocess.CREATE_NEW_PROCESS_GROUP
        if launch_type == "python":
            flags |= subprocess.CREATE_NO_WINDOW
        return {"creationflags": flags}
    return {"start_new_session": True}

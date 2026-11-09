"""Starting the hub: the server, then a pywebview window (or a browser tab as a fallback).

What happens at start-up is written to <data folder>/logs/hub.log, since Toki Hub.exe has no
console to print to.
"""
import logging
import threading
import time
import webbrowser
from pathlib import Path

from tokihub import __version__, paths
from tokihub.hub import Hub
from tokihub.server import HubServer

log = logging.getLogger("tokihub")

WINDOW = {"width": 1280, "height": 800, "min_size": (1000, 660), "background_color": "#EAF6FF"}

# Toki Hub.exe has no console, so when it has to fall back to a browser tab there is no Ctrl+C to
# stop it. It quits by itself once the page has been closed for this long instead. (Browsers can
# slow a hidden tab's polling to once a minute, so this must be comfortably longer than that.)
CLOSED_PAGE_TIMEOUT = 150


def run(browser=False, port=0, open_browser=True, data_dir=None, catalog_dir=None):
    hub = Hub(catalog_dir=catalog_dir, data_dir=data_dir)
    _start_log(hub.data_dir)
    log.info("Toki Hub %s starting (%s, catalogs: %s)", __version__,
             "Toki Hub.exe" if paths.is_frozen() else "from source", ", ".join(map(str, hub.catalog_dirs)))
    try:
        server = HubServer(hub, paths.ui_dir(), port)
    except OSError as e:
        _say(f"Toki Hub could not use port {port}: {e.strerror or e}. Pick another with --port, "
             "or leave --port out to use any free one.")
        return 1
    thread = threading.Thread(target=server.serve_forever, name="hub-server", daemon=True)
    thread.start()
    try:
        if not browser and _run_window(hub, server.url):
            log.info("Window closed")
            return 0
        _say(f"Toki Hub is running at {server.url}\nKeep this window open while you use it; press Ctrl+C to quit.")
        if open_browser:
            webbrowser.open(server.url)
        try:
            while thread.is_alive():
                thread.join(1)
                if paths.is_frozen() and _page_closed(server):
                    log.info("The page was closed, so Toki Hub is quitting")
                    break
        except KeyboardInterrupt:
            _say("\nToki Hub closed. See you soon, Sensei.")
        return 0
    finally:
        server.shutdown()
        server.server_close()


def _run_window(hub, url):
    """Show the hub in its own window. Returns False if that is not possible here."""
    try:
        import webview
    except ImportError:
        _say("pywebview is not installed, so Toki Hub will open in your web browser instead.\n"
             "For its own window: pip install pywebview")
        return False
    try:
        hub.window = webview.create_window("Toki Hub", url, **WINDOW)
        hub.window.events.shown += lambda *args: log.info("Window shown")
        log.info("Opening the window (pywebview %s)", getattr(webview, "__version__", "?"))
        webview.start()
        return True
    except Exception as e:      # no GUI backend available, etc.
        hub.window = None
        log.exception("The window could not be opened")
        _say(f"Could not open the Toki Hub window ({e}); opening it in your web browser instead.")
        return False


def _page_closed(server):
    last = server.last_api_call
    return last is not None and time.monotonic() - last > CLOSED_PAGE_TIMEOUT


def _say(message):
    print(message)          # does nothing in Toki Hub.exe, which has no console
    log.info(message.strip())


def _start_log(data_dir):
    try:
        logs = Path(data_dir) / "logs"
        logs.mkdir(parents=True, exist_ok=True)
        handler = logging.FileHandler(logs / "hub.log", mode="w", encoding="utf-8")
    except OSError:
        return
    handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(name)s: %(message)s"))
    root = logging.getLogger()
    root.addHandler(handler)
    root.setLevel(logging.INFO)

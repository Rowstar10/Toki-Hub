"""Starting the hub: the server, then a pywebview window (or a browser tab as a fallback)."""
import threading
import webbrowser

from tokihub import paths
from tokihub.hub import Hub
from tokihub.server import HubServer

WINDOW = {"width": 1280, "height": 800, "min_size": (1000, 660), "background_color": "#EAF6FF"}


def run(browser=False, port=0, open_browser=True, data_dir=None, catalog_dir=None):
    hub = Hub(catalog_dir=catalog_dir, data_dir=data_dir)
    try:
        server = HubServer(hub, paths.ui_dir(), port)
    except OSError as e:
        print(f"Toki Hub could not use port {port}: {e.strerror or e}. Pick another with --port, "
              "or leave --port out to use any free one.")
        return 1
    thread = threading.Thread(target=server.serve_forever, name="hub-server", daemon=True)
    thread.start()
    try:
        if not browser and _run_window(hub, server.url):
            return 0
        print(f"Toki Hub is running at {server.url}\nKeep this window open while you use it; press Ctrl+C to quit.")
        if open_browser:
            webbrowser.open(server.url)
        try:
            while thread.is_alive():
                thread.join(0.5)
        except KeyboardInterrupt:
            print("\nToki Hub closed. See you soon, Sensei.")
        return 0
    finally:
        server.shutdown()
        server.server_close()


def _run_window(hub, url):
    """Show the hub in its own window. Returns False if that is not possible here."""
    try:
        import webview
    except ImportError:
        print("pywebview is not installed, so Toki Hub will open in your web browser instead.\n"
              "For its own window: pip install pywebview")
        return False
    try:
        hub.window = webview.create_window("Toki Hub", url, **WINDOW)
        webview.start()
        return True
    except Exception as e:      # no GUI backend available, etc.
        hub.window = None
        print(f"Could not open the Toki Hub window ({e}); opening it in your web browser instead.")
        return False

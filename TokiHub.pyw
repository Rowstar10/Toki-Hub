"""Double-click this file (on Windows) to open Toki Hub in its own window, with no console.

It needs pywebview (pip install pywebview). Without it, use `python main.py`, which opens the hub
in your web browser instead.
"""
import sys

try:
    import webview  # noqa: F401
except ImportError:
    try:
        import tkinter
        from tkinter import messagebox
        tkinter.Tk().withdraw()
        messagebox.showinfo("Toki Hub", "Toki Hub needs pywebview to open its own window.\n\n"
                                        "Install it with:  pip install pywebview\n\n"
                                        "Or run  python main.py  to use your web browser instead.")
    except Exception:
        pass
    sys.exit(1)

from tokihub.cli import main

sys.exit(main([]))

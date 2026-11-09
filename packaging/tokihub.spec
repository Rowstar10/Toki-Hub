# -*- mode: python ; coding: utf-8 -*-
#
# Builds dist/Toki Hub.exe, a single file with the UI and the catalog inside it.
# On Windows, from the repository root:
#
#     pip install -r packaging/requirements.txt
#     pyinstaller --noconfirm packaging/tokihub.spec
#
# GitHub builds this automatically on every push (.github/workflows/build-exe.yml).
import os
import re
import sys

ROOT = os.path.dirname(SPECPATH)
VERSION = re.search(r'__version__ = "([^"]+)"', open(os.path.join(ROOT, "tokihub", "__init__.py")).read()).group(1)
NUMBERS = tuple((list(map(int, re.findall(r"\d+", VERSION)))[:4] + [0, 0, 0, 0])[:4])


def windows_version_info():
    """The details shown in the exe's Properties > Details tab."""
    from PyInstaller.utils.win32.versioninfo import (FixedFileInfo, StringFileInfo, StringStruct, StringTable,
                                                     VarFileInfo, VarStruct, VSVersionInfo)
    return VSVersionInfo(
        ffi=FixedFileInfo(filevers=NUMBERS, prodvers=NUMBERS, mask=0x3F, flags=0x0, OS=0x40004, fileType=0x1,
                          subtype=0x0, date=(0, 0)),
        kids=[
            StringFileInfo([StringTable("040904B0", [
                StringStruct("CompanyName", "Rowstar10"),
                StringStruct("FileDescription", "Toki Hub"),
                StringStruct("FileVersion", VERSION),
                StringStruct("InternalName", "Toki Hub"),
                StringStruct("OriginalFilename", "Toki Hub.exe"),
                StringStruct("ProductName", "Toki Hub"),
                StringStruct("ProductVersion", VERSION),
            ])]),
            VarFileInfo([VarStruct("Translation", [1033, 1200])]),
        ],
    )


a = Analysis(
    [os.path.join(ROOT, "main.py")],
    pathex=[ROOT],
    datas=[(os.path.join(ROOT, "ui"), "ui"), (os.path.join(ROOT, "catalog"), "catalog")],
    hiddenimports=[],
    # The hub only uses pywebview's Windows backend (WebView2), so leave the others out.
    excludes=["tkinter", "webview.platforms.qt", "webview.platforms.gtk", "webview.platforms.cocoa",
              "webview.platforms.android", "webview.platforms.cef", "qtpy", "PyQt5", "PyQt6", "PySide2",
              "PySide6", "gi"],
    noarchive=False,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="Toki Hub",
    icon=os.path.join(ROOT, "packaging", "tokihub.ico"),
    version=windows_version_info() if sys.platform == "win32" else None,
    console=False,          # a window app: no console behind it
    upx=False,              # UPX-packed exes are more often flagged by antivirus
    debug=False,
    strip=False,
    runtime_tmpdir=None,
)

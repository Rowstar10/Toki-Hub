# The catalog

Every game in Toki Hub is a folder in here, named after the game's id (lowercase letters, digits and
dashes), containing a `game.json` and its pictures:

```
catalog/
  hu-taong/
    game.json
    icon.png      square, shown in the library (256x256 is plenty)
    cover.jpg     wide banner at the top of the game's page (1280x720 works well)
```

After any change, run `python main.py check` to catch mistakes, then press the reload button in the hub
(or restart it).

- **New game:** `python main.py new-game "My Game"` creates the folder and a starter `game.json`.
- **New version:** add an entry to the game's `versions` list. Order doesn't matter: the hub sorts
  them newest first (`1.10` > `1.9` > `1.0` > `1.0-beta`) and badges the newest as *Latest*.

## game.json

```json
{
  "title": "Hu Taong",
  "tagline": "Pong, the Wangsheng Funeral Parlor way.",
  "description": "Longer text for the About panel.",
  "developer": "Rowstar10",
  "tags": ["Arcade", "Pong"],
  "accent": "#d94a3a",
  "icon": "icon.png",
  "cover": "cover.jpg",
  "versions": [ ... ]
}
```

Only `title` is required. `accent` is a colour for the stripe beside the game's title.

## Versions and builds

```json
{
  "version": "1.0",
  "date": "2026-11-03",
  "notes": "First release.\n- Lines starting with '- ' become bullet points",
  "builds": [
    {
      "platform": "windows",
      "source": { "type": "...", ... },
      "launch": { "type": "...", "path": "..." },
      "note": "Optional text shown under the Download/Launch button."
    }
  ]
}
```

A version can have several builds, one per platform: `windows`, `macos`, `linux` or `any`. The hub
uses the build for the computer it runs on, falling back to an `any` build. A version with no
matching build is still listed, marked "No Windows build".

Each version installs into its own folder, so any number of versions can be installed side by side.

### Where to download from: `source`

| `type` | Fields | Use it for |
| --- | --- | --- |
| `github-release` | `repo`, `tag`, `asset`, optional `sha256` | A file attached to a GitHub release. Works with private repos once a token is set in Settings. |
| `github-archive` | `repo`, `ref` (commit, tag or branch) | The repository's source code as it was at `ref`. Pin a commit so the version never changes. |
| `url` | `url`, optional `sha256`, `filename` | Any direct download link. |
| `local` | `path`, optional `sha256` | A folder, `.zip` or file on this computer (absolute, or relative to the game's catalog folder). |

Downloads ending in `.zip` are unpacked. If everything in the zip sits in one top folder (as in GitHub
source zips), that folder is skipped, so `main.py` ends up directly in the version's folder. Any other
file (such as a single `.exe`) is kept as it is.

`sha256` is optional. When it is set, a download that doesn't match is refused. To get a file's
checksum on Windows: `Get-FileHash .\HuTaong-1.1-windows.zip -Algorithm SHA256` in PowerShell.

### How to start it: `launch`

`path` is relative to the version's installed folder. The game starts with its own folder as the
working directory, and its output goes to a log (the hub's **Log** button).

| `type` | Runs |
| --- | --- |
| `executable` | the program at `path`, e.g. `main.exe`, plus any `args` |
| `python` | `path` with the Python set in Settings (by default the one running the hub), with no console window |
| `open` | `path` with whatever the system opens it with, e.g. an `index.html` in the browser |

## Recipes

### A Windows .exe from a GitHub release (recommended for finished versions)

1. Build the game, e.g. `pyinstaller main.spec` for Hu Taong, which makes `dist/main.exe`.
2. On GitHub, open the game's repository, then **Releases > Draft a new release**. Make a tag such as
   `v1.0` and attach `main.exe`, or a zip of the `dist` folder.
3. Add a build to that version:

```json
{
  "platform": "windows",
  "source": { "type": "github-release", "repo": "Rowstar10/Hu-Taong-v1.0", "tag": "v1.0", "asset": "main.exe" },
  "launch": { "type": "executable", "path": "main.exe" }
}
```

Windows players get the .exe, which needs no Python. Other platforms keep using the version's `any`
build, if it has one.

### A version straight from its source code

This is how Hu Taong 1.0 is listed right now. Players need Python with the game's packages installed.

```json
{
  "platform": "any",
  "source": { "type": "github-archive", "repo": "Rowstar10/Hu-Taong-v1.1", "ref": "<full commit hash>" },
  "launch": { "type": "python", "path": "main.py" },
  "note": "Runs from source: needs Python 3 with pygame (pip install pygame)."
}
```

### An old version that only exists on this computer

```json
{
  "platform": "windows",
  "source": { "type": "local", "path": "D:/Old Games/Hu Taong 0.9" },
  "launch": { "type": "executable", "path": "main.exe" }
}
```

### A whole new game

```
python main.py new-game "Toki's Adventure"
```

Then add `icon.png`, `cover.jpg` and a version as above, and run `python main.py check`.

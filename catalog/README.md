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

After any change, press the reload button in the hub: mistakes show up as catalog problems in the
library. GitHub also checks the whole catalog on every push (the build fails if anything is wrong).

- **New game:** copy the `hu-taong` folder, rename it, and edit its `game.json` and pictures.
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

## Changelog

A game can point to a changelog kept with its code, and each version's page shows that version's
part of it instead of its `notes`:

```json
"changelog": { "repo": "Rowstar10/Hu-Taong", "path": "CHANGELOG.md" }
```

`path` defaults to `CHANGELOG.md`, and `ref` (a branch, tag or commit) to the repository's default
branch. Instead of `repo`, `url` can be any direct link to the file. Private repositories need the
GitHub token in Settings, like downloads do.

The changelog is Markdown with a `## ` heading for each version; the heading's first word is the
version (`## 1.3 — 2026-11-10`, `## v1.3` and `## [1.3] - 2026-11-10` all work). Under it, `### `
lines become small headings and lines starting with `- ` become bullet points:

```markdown
## 1.3 — 2026-11-10

One line saying what the version is about.

### Added
- Online play
```

The hub fetches it once per session (again when you press reload) and keeps the last copy, so it
still shows offline. Editing the file on GitHub is enough: no new hub release is needed. A version
missing from the changelog, or a changelog that can't be fetched and was never saved, falls back
to the version's `notes`, so keep a one-line `notes` for each version as well.

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

A version can have several builds, one per platform: `windows`, `macos`, `linux`, `android` or
`any`. The hub uses the build for the computer it runs on, falling back to an `any` build. A version
with no matching build is still listed, marked "No Windows build". Toki Hub on a phone only ever uses
`android` builds (an `any` build is for computers).

Each version installs into its own folder, so any number of versions can be installed side by side.

### Where to download from: `source`

| `type` | Fields | Use it for |
| --- | --- | --- |
| `github-release` | `repo`, `tag`, `asset`, optional `sha256` | A file attached to a GitHub release. |
| `github-archive` | `repo`, `ref` (commit, tag or branch) | The repository's source code as it was at `ref`. Pin a commit so the version never changes. |
| `url` | `url`, optional `sha256`, `filename` | Any direct download link. |
| `local` | `path`, optional `sha256` | A folder, `.zip` or file on this computer (absolute, or relative to the game's catalog folder). |

GitHub sources from **public** repositories need no token. Private repositories need a GitHub token in
Settings (or the repository made public).

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

Python games can also say what they need:

```json
"launch": { "type": "python", "path": "main.py", "python": "3.12", "packages": ["pygame==2.6.1"] }
```

Toki Hub installs that Python and those packages for the game when it is installed (a private copy,
nothing system-wide). Pin package versions (`==`) so old versions keep working as packages change.

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

### An Android version (for Toki Hub on phones)

```json
{
  "platform": "android",
  "source": { "type": "github-release", "repo": "Rowstar10/Hu-Taong", "tag": "v1.5-android", "asset": "Hu-Taong-1.5-android.apk" },
  "launch": { "type": "android", "package": "com.rowstar10.hutaong.v1_5" }
}
```

On Android a game is an app: the hub downloads the APK and Android's installer asks the player to
install it. `package` is the app's package name, which is how the hub knows it is installed and
starts it. Give each version its own package name so versions can be installed side by side.

### An old version that only exists on this computer

```json
{
  "platform": "windows",
  "source": { "type": "local", "path": "D:/Old Games/Hu Taong 0.9" },
  "launch": { "type": "executable", "path": "main.exe" }
}
```

### A whole new game

Copy `catalog/hu-taong` to a new folder named after the game (like `catalog/tokis-adventure`), then change
the title, pictures and versions in it. Press reload in the hub to see it.

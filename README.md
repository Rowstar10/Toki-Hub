# Toki Hub

A launcher for all of my games and every version of them, looked after by Asuma Toki
(C&C call sign 04). Pick a game, pick a version, download it, launch it.

![Toki Hub](docs/screenshot.jpg)

- **Every version, side by side.** Each version installs into its own folder, so old and new
  versions can be installed and played at the same time.
- **Downloads from GitHub,** including private repositories: release files (such as a PyInstaller
  `.exe`) or a repository's source code at a pinned commit. Plain links and files already on this
  computer work too.
- **Adding a game is a folder and a JSON file.** No code changes. See [catalog/README.md](catalog/README.md).
- **Safe installs:** downloads are checked against an optional sha256, zips can't write outside
  their folder, and a failed or cancelled download never leaves a half-installed version behind.
- **Crash reports:** if a game closes unexpectedly, the hub shows the end of its log (e.g. a missing
  `pygame`) and keeps the full log one click away.

## Running it

Needs Python 3.9 or newer.

```bash
pip install -r requirements.txt
```

```bash
python main.py
```

On Windows you can instead double-click **`TokiHub.pyw`** to open the hub without a console window.

`requirements.txt` only contains [pywebview](https://pywebview.flowrl.com/), which gives the hub its
own window. Without it, `python main.py` opens the hub in your web browser instead. `python main.py --browser`
does that on purpose.

### First-time setup

1. **GitHub token.** My game repositories are private, so the hub needs a token to download from
   them. On GitHub go to *Settings > Developer settings > Fine-grained tokens*, create one with
   read-only **Contents** access to the game repositories, and paste it into the hub's
   Settings (the sliders button, top right). It stays on this computer. Alternatively, set it in the
   `GITHUB_TOKEN` environment variable.
2. **Python games.** Hu Taong 1.0 is listed as source code, so it runs with the same Python as the hub
   and needs `pip install pygame`. Once a version has a Windows `.exe` in a GitHub release, that
   build is used instead and needs no Python (see the recipe in [catalog/README.md](catalog/README.md)).

## Adding games and versions

Everything the hub knows about lives in [`catalog/`](catalog), one folder per game:

```bash
python main.py new-game "My New Game"
```

Then fill in its `game.json`, then check your work:

```bash
python main.py check
```

[catalog/README.md](catalog/README.md) explains every field and has copy-paste examples for each kind
of download.

## Toki's portrait

The dialogue bar shows an original badge (bunny ears, a halo and "04"). To show a picture of Toki
instead, save it as `ui/img/toki.png`. It's git-ignored, so fan art stays on your computer. Her
lines are in [`ui/js/toki.js`](ui/js/toki.js). Add more there, and one is picked at random each time.

## Where things are kept

| What | Where |
| --- | --- |
| Settings, logs, downloads in progress | `%LOCALAPPDATA%\TokiHub` on Windows, `~/.local/share/TokiHub` on Linux, `~/Library/Application Support/TokiHub` on macOS |
| Installed games | `<that folder>\games\<game>\<version>`, or the install folder chosen in Settings |

Games keep their own save data wherever they normally do (Hu Taong uses `%APPDATA%\HuTaong`), so it is
shared between versions and survives uninstalling.

## Project layout

```
main.py, TokiHub.pyw   start the hub
catalog/               the games (data only)
tokihub/               the hub itself, standard library only
  catalog.py           reads and checks catalog/*/game.json
  sources.py           turns a build's source into a download (GitHub API, URLs, local files)
  installer.py         downloads, verifies and unpacks versions
  launcher.py          starts games, captures their logs, notices crashes
  hub.py               everything the UI can ask for
  server.py            serves the UI on 127.0.0.1 (each page gets a secret token)
  app.py, cli.py       the window and the command line
ui/                    the interface (plain HTML, CSS and JavaScript, no build step)
tests/                 python -m unittest
```

## Tests

```bash
python -m unittest
```

## Credits

- Asuma Toki and Blue Archive belong to NEXON Games and Yostar. This is a personal fan project; the
  emblem and Toki's lines here are original.
- Hu Taong's icon and cover come from the game's own resources (see its `CREDITS.md`).
- Fonts: [Exo 2](https://github.com/googlefonts/Exo-2.0) and [Nunito](https://github.com/googlefonts/nunito),
  both under the SIL Open Font License (texts in `ui/fonts`).

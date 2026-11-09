# Toki Hub

A launcher for all my games and every version of them.

![Toki Hub](docs/screenshot.jpg)

## Download

Get **Toki Hub.exe** from the [latest release](https://github.com/Rowstar10/Toki-Hub/releases/latest) and
double-click it. Nothing to install.

Windows may say it "protected your PC", because the exe isn't code-signed. Click **More info**, then
**Run anyway**.

## Setup

- **GitHub token:** the game repositories are private, so open Settings (top right) and paste in a
  [fine-grained token](https://github.com/settings/personal-access-tokens/new) with read-only
  **Contents** access to them.
- **Hu Taong 1.0** runs from its source code, so it needs Python with pygame: `pip install pygame`.

## Adding games and versions

Each game is a folder in [`catalog/`](catalog) with a `game.json`. [catalog/README.md](catalog/README.md)
explains the format and has copy-paste examples. Games added there are built into the next release.

To add a game without making a release, put its folder in `%LOCALAPPDATA%\Toki Hub\catalog` and press
the reload button in the hub.

## Releasing a new version of the hub

Change `__version__` in [`tokihub/__init__.py`](tokihub/__init__.py) and push. GitHub builds Toki Hub.exe
and publishes it as a new release.

## Running from source

```bash
pip install -r requirements.txt
python main.py
```

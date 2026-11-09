# Toki Hub

A launcher for all my games and every version of them.

![Toki Hub](docs/screenshot.jpg)

## Download

Get **Toki Hub.exe** from the [latest release](https://github.com/Rowstar10/Toki-Hub/releases/latest) and
double-click it. Nothing to install.

Windows may say it "protected your PC", because the exe isn't code-signed. Click **More info**, then
**Run anyway**.

## Setup

Open Settings (top right) and paste in a
[fine-grained GitHub token](https://github.com/settings/personal-access-tokens/new) with read-only
**Contents** access to this repository and the game repositories. They are private, so the hub needs it
to download games and its own updates.

## What it does

- **Installs games with everything they need.** Python games get the Python version and packages their
  catalog entry lists, installed just for that game: nothing system-wide, no admin rights.
- **Updates itself.** When a new version is out, a gold **Update** button appears at the top.
  Settings › Updates has **Check now**.
- Settings and installed games are in `%LOCALAPPDATA%\Toki Hub (Tauri)`.

## Adding games and versions

Each game is a folder in [`catalog/`](catalog) with a `game.json`. [catalog/README.md](catalog/README.md)
explains the format and has copy-paste examples. Games added there are built into the next release.

To add a game without making a release, put its folder in `%LOCALAPPDATA%\Toki Hub (Tauri)\catalog` and
press the reload button in the hub.

## Releasing a new version

Change `version` in [`src-tauri/Cargo.toml`](src-tauri/Cargo.toml) and push. GitHub builds Toki Hub.exe,
publishes it as a release, and running copies offer the update.

## Building it yourself

Needs [Rust](https://rustup.rs) and [Node.js](https://nodejs.org):

```bash
npm ci
npm run build
```

The exe is `target/release/toki-hub.exe`. Tests:

```bash
cargo test -p toki-hub-core
```

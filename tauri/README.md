# Toki Hub (Tauri)

The Tauri version of Toki Hub: the same hub and interface, with the Python backend rewritten in Rust.
It reads the same [`catalog/`](../catalog) as the Python version, but keeps its own settings and
installed games, so the two never interfere.

## Download

Get **Toki Hub.exe** from the [Tauri pre-releases](https://github.com/Rowstar10/Toki-Hub/releases)
(tagged `tauri-v…`) and double-click it.

Windows may say it "protected your PC", because the exe isn't code-signed. Click **More info**, then
**Run anyway**.

## Differences from the Python version

- **It updates itself.** A few seconds after it starts (and every six hours) it checks for a newer
  `tauri-v…` release. If there is one, a gold **Update** button appears at the top: it downloads the
  new exe, checks it against the checksum GitHub records for it, swaps it in and restarts. Settings ›
  Updates shows the status and has **Check now**. The repository is private, so this needs the GitHub
  token in Settings, with access to this repository as well as the game repositories.
- **Installing a Python game installs what it needs.** The Python version and packages in its catalog
  entry (`"python"`, `"packages"`) are installed just for that game, using
  [uv](https://docs.astral.sh/uv/): nothing system-wide, nothing added to the PATH, no admin rights.
  Uninstalling a game removes its packages (the Python itself is kept for other games). If a Python is
  set in Settings, that is used instead.
- Settings and installed games are in `%LOCALAPPDATA%\Toki Hub (Tauri)`.
- To add a game without a new release, put its folder in `%LOCALAPPDATA%\Toki Hub (Tauri)\catalog`.

## Releasing a new version

Change `version` in [`src-tauri/Cargo.toml`](src-tauri/Cargo.toml) and push. GitHub builds the exe and
publishes it as a pre-release.

## Building it yourself

Needs [Rust](https://rustup.rs) and [Node.js](https://nodejs.org). From this folder:

```bash
npm ci
npm run build
```

The exe is `target/release/toki-hub.exe`. Run the tests with:

```bash
cargo test -p toki-hub-core
```

## Layout

- `core/`: the hub's logic (catalog, downloads, installs, launching), with its tests
- `src-tauri/`: the Tauri app that connects it to the window
- `ui/`: the interface, a copy of the Python version's

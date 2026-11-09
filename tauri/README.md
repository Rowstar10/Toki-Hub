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

- Settings and installed games are in `%LOCALAPPDATA%\Toki Hub (Tauri)`.
- To add a game without a new release, put its folder in `%LOCALAPPDATA%\Toki Hub (Tauri)\catalog`.
- Python games use the `py` launcher on Windows, unless another Python is set in Settings.

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

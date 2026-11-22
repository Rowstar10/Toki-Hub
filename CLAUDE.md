# Toki Hub

- The main branch is `Toki-Hub`. This repo is private; friends get Toki Hub from the public repo
  Rowstar10/Toki-Hub-Releases (UPDATE_REPO in src-tauri/src/lib.rs).
- Games install from public releases repos named in catalog/<game>/game.json ("releases").
  No GitHub token or secret goes in the app or in any releases repo.
- The launcher never shows GitHub repos, usernames or where a download comes from.
- Release: add a CHANGELOG.md section, bump the version in src-tauri/Cargo.toml (and Cargo.lock),
  push, wait for CI to pass, then run `py release.py`.
- Installs download to a temp file, check its sha256, and replace the old copy only after a
  successful extract (core/src/installer.rs). Keep it that way.
- Tests: `cargo test` in core/. Checks against the real releases: `cargo test -- --ignored real_`.
- Don't commit build output.

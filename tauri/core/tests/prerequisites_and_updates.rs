//! Installing a game's Python and packages, and updating the hub itself.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use common::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use toki_hub_core::catalog::Catalog;
use toki_hub_core::runtime::{env_python, wheel_tag, ENV_DIR, UV_VERSION};
use toki_hub_core::updates::{old_exe, remove_old_exe, swap_exe, UpdateSource};
use toki_hub_core::{Hub, HubOptions};

fn sha(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// A hub that sets up Python itself (nothing set in Settings), with PyPI and GitHub faked.
fn self_reliant_hub(world: &World) -> Hub {
    let mut options = HubOptions::new(vec![world.catalog()], world.data());
    options.github_api = world.web.base.clone();
    options.pypi = world.web.base.clone();
    Hub::new(options)
}

fn python_game(world: &World, versions: &[(&str, &[&str])]) {
    let build = world.dir.path().join("build");
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(build.join("main.py"), "import sys\nprint('running on', sys.executable)\n").unwrap();
    let versions = versions
        .iter()
        .map(|(version, packages)| {
            json!({"version": version, "builds": [{"platform": "any",
                "source": {"type": "local", "path": build.to_string_lossy()},
                "launch": {"type": "python", "path": "main.py", "python": "3.12", "packages": packages}}]})
        })
        .collect();
    write_game(&world.catalog(), "game", &game_with(versions));
}

// ---------- Catalog ----------

#[test]
fn python_and_packages_are_checked() {
    let world = World::new();
    let launch = |extra: Value| {
        let mut launch = json!({"type": "python", "path": "main.py"});
        launch.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        launch
    };
    write_game(&world.catalog(), "game", &game_with(vec![
        version_launching("1.0", json!({"type": "url", "url": "https://x/y.zip"}), launch(json!({"python": "3.12", "packages": ["pygame==2.6.1"]}))),
        version_launching("1.1", json!({"type": "url", "url": "https://x/y.zip"}), launch(json!({"python": "three"}))),
        version_launching("1.2", json!({"type": "url", "url": "https://x/y.zip"}), launch(json!({"packages": ["--index-url=https://evil.example"]}))),
        version_launching("1.3", json!({"type": "url", "url": "https://x/y.zip"}),
                          json!({"type": "executable", "path": "game.exe", "packages": ["pygame"]})),
    ]));
    let catalog = Catalog::load(&[world.catalog()]);
    let game = catalog.game("game").unwrap();
    let ok = &game.version("1.0").unwrap().builds[0].launch;
    assert_eq!((ok.python.as_str(), ok.packages.clone()), ("3.12", vec!["pygame==2.6.1".to_string()]));
    assert!(game.version("1.1").unwrap().builds.is_empty());
    assert!(game.version("1.2").unwrap().builds.is_empty(), "options must not sneak in as packages");
    assert_eq!(game.version("1.3").unwrap().builds.len(), 1, "a misplaced field is only a warning");
    let problems = catalog.problems.join("\n");
    for expected in ["'python' must be a version like", "'packages' must be a list of package names", "only apply to python games"] {
        assert!(problems.contains(expected), "missing {expected:?} in:\n{problems}");
    }
}

// ---------- Prerequisites, with uv faked ----------

/// A stand-in for uv that records how it was called and makes a working environment.
#[cfg(unix)]
fn serve_fake_uv(world: &World) {
    let python = python().expect("these tests need python3");
    let script = format!(
        r#"#!/bin/sh
mkdir -p "$UV_CACHE_DIR"
echo "$* | $UV_PYTHON_INSTALL_DIR | certs=$UV_SYSTEM_CERTS | $UV_PYTHON_PREFERENCE | bin=$UV_PYTHON_INSTALL_BIN registry=$UV_PYTHON_INSTALL_REGISTRY" >> "$UV_CACHE_DIR/calls.log"
for last; do :; done
case "$1" in
  python) echo "Installed Python $3" ;;
  venv) mkdir -p "$last/bin" && printf '#!/bin/sh\nexec {python} "$@"\n' > "$last/bin/python" && chmod +x "$last/bin/python" ;;
  pip) env="$4"; shift 4
       if [ "$1" = "broken==1" ]; then echo "error: No solution found when resolving dependencies: broken==1" >&2; exit 1; fi
       echo "$@" > "$env/installed.txt" ;;
esac
"#
    );
    let wheel = zip_bytes(&[(&format!("uv-{UV_VERSION}.data/scripts/uv"), &script)], None);
    let tag = wheel_tag().expect("a supported computer");
    world.web.json(
        &format!("/pypi/uv/{UV_VERSION}/json"),
        json!({"urls": [
            {"filename": format!("uv-{UV_VERSION}-py3-none-other_platform.whl"), "url": "nope", "digests": {"sha256": "0"}},
            {"filename": format!("uv-{UV_VERSION}-py3-none-{tag}.whl"), "url": format!("{}/files/uv.whl", world.web.base),
             "digests": {"sha256": sha(&wheel)}},
        ]}),
    );
    world.web.file("/files/uv.whl", wheel);
}

#[cfg(unix)]
#[test]
fn install_sets_up_the_games_own_python_and_packages() {
    let world = World::new();
    serve_fake_uv(&world);
    python_game(&world, &[("1.0", &["pygame==2.6.1"]), ("2.0", &[])]);
    let hub = self_reliant_hub(&world);
    let task = install_and_wait(&hub, "game", "1.0");
    assert_eq!(task["status"], "done", "{task}");
    let folder = hub.installer.version_dir("game", "1.0");
    assert_eq!(std::fs::read_to_string(folder.join(ENV_DIR).join("installed.txt")).unwrap().trim(), "pygame==2.6.1");

    let calls = std::fs::read_to_string(world.data().join("cache/uv/calls.log")).unwrap();
    let runtimes = world.data().join("runtimes/python");
    let expected = format!("python install 3.12 | {} | certs=1 | only-managed | bin=0 registry=0", runtimes.display());
    assert!(calls.contains(&expected), "nothing may be installed outside the hub's folder:\n{calls}");
    assert!(calls.contains("venv --relocatable --no-project --python 3.12"), "{calls}");
    assert!(calls.contains("pip install --python"), "{calls}");

    // The game runs with its own Python.
    hub.launch("game", "1.0").unwrap();
    wait_for("the game to exit", || hub.poll(0)["events"].as_array().unwrap().first().cloned());
    let log = std::fs::read_to_string(hub.launcher.log_path("game", "1.0")).unwrap();
    assert!(log.contains(&env_python(&folder.join(ENV_DIR)).display().to_string()), "{log}");

    // uv is downloaded once, and a game without packages still gets its Python.
    assert_eq!(install_and_wait(&hub, "game", "2.0")["status"], "done");
    assert!(env_python(&hub.installer.version_dir("game", "2.0").join(ENV_DIR)).is_file());
    assert_eq!(world.web.headers_for("/files/uv.whl").len(), 1);
}

#[cfg(unix)]
#[test]
fn a_failed_package_install_explains_itself_and_installs_nothing() {
    let world = World::new();
    serve_fake_uv(&world);
    python_game(&world, &[("1.0", &["broken==1"])]);
    let hub = self_reliant_hub(&world);
    let task = install_and_wait(&hub, "game", "1.0");
    assert_eq!(task["status"], "error");
    let error = task["error"].as_str().unwrap();
    assert!(error.contains("Could not install broken") && error.contains("No solution found"), "{error}");
    assert!(!hub.installer.version_dir("game", "1.0").exists());
}

#[test]
fn a_python_chosen_in_settings_skips_the_setup() {
    let world = World::new();
    python_game(&world, &[("1.0", &["pygame==2.6.1"])]);
    let hub = world.hub(); // sets a Python in Settings
    assert_eq!(install_and_wait(&hub, "game", "1.0")["status"], "done");
    assert!(!hub.installer.version_dir("game", "1.0").join(ENV_DIR).exists());
    assert!(world.web.headers_for(&format!("/pypi/uv/{UV_VERSION}/json")).is_empty());
}

/// The real thing: uv from PyPI, Python from python-build-standalone, pygame from PyPI.
/// Needs the internet, so it only runs when asked: cargo test -- --ignored real_
#[test]
#[ignore]
fn real_python_and_pygame_install_and_run() {
    let world = World::new();
    let build = world.dir.path().join("build");
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(build.join("main.py"), "import pygame\nprint('pygame works:', pygame.version.ver)\n").unwrap();
    write_game(&world.catalog(), "game", &game_with(vec![json!({"version": "1.0", "builds": [{"platform": "any",
        "source": {"type": "local", "path": build.to_string_lossy()},
        "launch": {"type": "python", "path": "main.py", "python": "3.12", "packages": ["pygame==2.6.1"]}}]})]));
    let hub = Hub::new(HubOptions::new(vec![world.catalog()], world.data()));
    let task = install_and_wait_long(&hub, "game", "1.0");
    let install_log = std::fs::read_to_string(world.data().join("logs/game-1.0-install.log")).unwrap_or_default();
    assert_eq!(task["status"], "done", "{task}\n{install_log}");
    hub.launch("game", "1.0").unwrap();
    let event = wait_for("the game to exit", || hub.poll(0)["events"].as_array().unwrap().first().cloned());
    let log = std::fs::read_to_string(hub.launcher.log_path("game", "1.0")).unwrap();
    assert_eq!(event["code"].as_i64(), Some(0), "{log}");
    assert!(log.contains("pygame works: 2.6.1"), "{log}");
}

fn install_and_wait_long(hub: &Hub, game: &str, version: &str) -> Value {
    hub.install(game, version).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(600);
    loop {
        let task = hub.installer.tasks().into_iter().next().unwrap();
        if !matches!(task["status"].as_str(), Some("queued" | "downloading" | "installing")) || std::time::Instant::now() > deadline {
            return task;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ---------- Updating the hub ----------

fn release(tag: &str, assets: Value) -> Value {
    json!({"tag_name": tag, "draft": false, "prerelease": true, "body": format!("Notes for {tag}"), "assets": assets})
}

fn updating_hub(world: &World, current: &str) -> Hub {
    let mut options = HubOptions::new(vec![world.catalog()], world.data());
    options.github_api = world.web.base.clone();
    options.hub_version = current.into();
    options.updates = Some(UpdateSource { repo: "Owner/Hub".into(), tag_prefix: "tauri-v".into(), asset_suffix: ".exe".into() });
    Hub::new(options)
}

#[test]
fn finds_downloads_and_verifies_the_newest_release() {
    let world = World::new();
    let new_exe = b"the new Toki Hub".to_vec();
    let asset = |name: &str, path: &str, digest: Option<String>| {
        let mut asset = json!({"name": name, "url": format!("{}{path}", world.web.base), "size": new_exe.len()});
        if let Some(digest) = digest {
            asset["digest"] = json!(format!("sha256:{digest}"));
        }
        asset
    };
    let mut draft = release("tauri-v0.5.0", json!([asset("Toki.Hub.exe", "/assets/5", Some(sha(b"x")))]));
    draft["draft"] = json!(true);
    world.web.json("/repos/Owner/Hub/releases?per_page=50", json!([
        release("v9.0.0", json!([asset("Toki.Hub.exe", "/assets/python", Some(sha(b"x")))])),
        release("tauri-v0.4.0", json!([asset("Toki.Hub.exe", "/assets/unverifiable", None)])),
        release("tauri-v0.3.0", json!([asset("notes.txt", "/assets/notes", Some(sha(b"x")))])),
        release("tauri-v0.2.0", json!([asset("Toki.Hub.exe", "/assets/2", Some(sha(&new_exe)))])),
        release("tauri-v0.1.0", json!([asset("Toki.Hub.exe", "/assets/1", Some(sha(b"old")))])),
        draft,
    ]));
    world.web.file("/assets/2", new_exe.clone());
    let hub = updating_hub(&world, "0.1.0");
    hub.save_settings(&json!({"github_token": "secret-token"})).unwrap();
    let found = hub.check_for_updates();
    assert_eq!((found["status"].as_str(), found["version"].as_str()), (Some("available"), Some("0.2.0")), "{found}");
    assert_eq!(found["notes"], "Notes for tauri-v0.2.0");

    hub.download_update().unwrap();
    let file = wait_for("the update to download", || hub.update_file());
    assert_eq!(std::fs::read(&file).unwrap(), new_exe);
    let headers = &world.web.headers_for("/assets/2")[0];
    assert_eq!(headers.get("authorization").map(String::as_str), Some("Bearer secret-token"));
    assert_eq!(headers.get("accept").map(String::as_str), Some("application/octet-stream"));
    assert_eq!(hub.poll(0)["update"]["status"], "ready");
}

#[test]
fn a_tampered_update_is_refused() {
    let world = World::new();
    world.web.json("/repos/Owner/Hub/releases?per_page=50", json!([release("tauri-v0.2.0",
        json!([{"name": "Toki.Hub.exe", "url": format!("{}/assets/2", world.web.base), "digest": format!("sha256:{}", sha(b"genuine"))}]))]));
    world.web.file("/assets/2", b"something else".to_vec());
    let hub = updating_hub(&world, "0.1.0");
    assert_eq!(hub.check_for_updates()["status"], "available");
    hub.download_update().unwrap();
    let update = wait_for("the download to fail", || {
        let update = hub.poll(0)["update"].clone();
        (update["status"] == "error").then_some(update)
    });
    assert!(update["error"].as_str().unwrap().contains("does not match its sha256"), "{update}");
    assert!(hub.update_file().is_none());
}

#[test]
fn up_to_date_or_unreachable() {
    let world = World::new();
    world.web.json("/repos/Owner/Hub/releases?per_page=50", json!([release("tauri-v0.2.0",
        json!([{"name": "Toki.Hub.exe", "url": "x", "digest": format!("sha256:{}", sha(b"x"))}]))]));
    assert_eq!(updating_hub(&world, "0.2.0").check_for_updates()["status"], "current");

    let private = World::new(); // no releases route: GitHub says 404, as it does for a private repo without a token
    let result = updating_hub(&private, "0.1.0").check_for_updates();
    assert_eq!(result["status"], "error");
    assert!(result["error"].as_str().unwrap().contains("add a GitHub token"), "{result}");

    let mut options = HubOptions::new(vec![private.catalog()], private.data());
    options.updates = None;
    assert_eq!(Hub::new(options).check_for_updates()["status"], "unsupported");
}

/// Not a real test: a program for `swapping_the_running_exe` to run (a copy of this test binary).
#[test]
#[ignore]
fn helper_wait_a_while() {
    std::thread::sleep(Duration::from_secs(8));
}

#[test]
fn swapping_the_running_exe() {
    let dir = TempDir::new();
    let app = dir.path().join(if cfg!(windows) { "Toki Hub.exe" } else { "Toki Hub" });
    std::fs::copy(std::env::current_exe().unwrap(), &app).unwrap();
    let mut running = Command::new(&app)
        .args(["helper_wait_a_while", "--exact", "--ignored"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(running.try_wait().unwrap().is_none(), "the copy should still be running");

    let update: PathBuf = dir.path().join("update-0.2.0");
    std::fs::write(&update, b"the new version").unwrap();
    swap_exe(&app, &update).unwrap();
    assert_eq!(std::fs::read(&app).unwrap(), b"the new version");
    assert!(old_exe(&app).exists() && !update.exists());

    let _ = running.kill();
    let _ = running.wait();
    assert!(remove_old_exe(&app, Duration::from_secs(10)));
    assert!(!old_exe(&app).exists());
}

#[test]
fn a_failed_swap_puts_the_old_exe_back() {
    let dir = TempDir::new();
    let app = dir.path().join("Toki Hub.exe");
    std::fs::write(&app, b"current").unwrap();
    let missing: &Path = &dir.path().join("no-such-update");
    assert!(swap_exe(&app, missing).is_err());
    assert_eq!(std::fs::read(&app).unwrap(), b"current");
}

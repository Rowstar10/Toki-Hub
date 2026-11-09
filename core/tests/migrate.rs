//! Moving the data folder from "Toki Hub (Tauri)" to "Toki Hub".

mod common;

use std::path::Path;

use common::*;
use serde_json::json;
use toki_hub_core::migrate::{settle_data_dir, PYTHON_LEFTOVERS};
use toki_hub_core::{Hub, HubOptions};

fn hub_data(dir: &Path) {
    std::fs::create_dir_all(dir.join("cache/catalog")).unwrap();
    std::fs::write(dir.join("settings.json"), r#"{"github_token": "secret"}"#).unwrap();
}

#[test]
fn the_old_folder_is_renamed() {
    let base = TempDir::new();
    let old = base.path().join("Toki Hub (Tauri)");
    hub_data(&old);
    let (used, log) = settle_data_dir(&base.path().join("Toki Hub"));
    assert_eq!(used, base.path().join("Toki Hub"));
    assert!(!old.exists());
    assert!(used.join("settings.json").is_file(), "settings, including the token, carry over");
    assert!(log[0].starts_with("Renamed"), "{log:?}");
}

#[test]
fn the_python_versions_leftovers_are_kept_inside() {
    let base = TempDir::new();
    let python = base.path().join("Toki Hub");
    std::fs::create_dir_all(python.join("games/hu-taong/1.0")).unwrap();
    std::fs::write(python.join("settings.json"), "{}").unwrap();
    hub_data(&base.path().join("Toki Hub (Tauri)"));
    let (used, _) = settle_data_dir(&python);
    assert_eq!(used, python);
    assert!(used.join("cache/catalog").is_dir(), "the hub's data is now in Toki Hub");
    assert!(used.join(PYTHON_LEFTOVERS).join("games/hu-taong/1.0").is_dir(), "nothing was lost");
    assert_eq!(listing(base.path()), ["Toki Hub"]);
}

#[test]
fn nothing_moves_when_the_new_folder_is_already_in_use() {
    let base = TempDir::new();
    hub_data(&base.path().join("Toki Hub"));
    hub_data(&base.path().join("Toki Hub (Tauri)"));
    let (used, _) = settle_data_dir(&base.path().join("Toki Hub"));
    assert_eq!(used, base.path().join("Toki Hub"));
    assert!(base.path().join("Toki Hub (Tauri)").is_dir());
    assert!(!base.path().join("Toki Hub").join(PYTHON_LEFTOVERS).exists());
}

#[test]
fn nothing_happens_without_an_old_folder() {
    let base = TempDir::new();
    let (used, log) = settle_data_dir(&base.path().join("Toki Hub"));
    assert_eq!(used, base.path().join("Toki Hub"));
    assert!(log.is_empty() && listing(base.path()).is_empty());
}

/// The shapes uv leaves behind on Linux and macOS: an absolute "3.12" symlink, an environment whose
/// pyvenv.cfg and bin/python point through it.
#[cfg(unix)]
#[test]
fn python_shortcuts_and_environment_links_follow_the_move() {
    use std::os::unix::fs::symlink;
    let base = TempDir::new();
    let old = base.path().join("Toki Hub (Tauri)");
    hub_data(&old);
    let runtimes = old.join("runtimes/python");
    std::fs::create_dir_all(runtimes.join("cpython-3.12.15-linux/bin")).unwrap();
    std::fs::write(runtimes.join("cpython-3.12.15-linux/bin/python3.12"), "").unwrap();
    symlink(runtimes.join("cpython-3.12.15-linux"), runtimes.join("cpython-3.12-linux")).unwrap();
    let env = old.join("games/game/1.0/.toki-python");
    std::fs::create_dir_all(env.join("bin")).unwrap();
    let home = runtimes.join("cpython-3.12-linux/bin");
    std::fs::write(env.join("pyvenv.cfg"), format!("home = {}\nversion_info = 3.12\n", home.display())).unwrap();
    symlink(home.join("python3.12"), env.join("bin/python")).unwrap();
    symlink("python", env.join("bin/python3")).unwrap();

    let (new, log) = settle_data_dir(&base.path().join("Toki Hub"));
    let env = new.join("games/game/1.0/.toki-python");
    let real_bin = new.join("runtimes/python/cpython-3.12.15-linux/bin");
    let config = std::fs::read_to_string(env.join("pyvenv.cfg")).unwrap();
    assert!(config.contains(&format!("home = {}", real_bin.display())), "{config}\n{log:?}");
    assert!(config.contains("version_info = 3.12"), "other lines are kept");
    assert_eq!(std::fs::read_link(env.join("bin/python")).unwrap(), real_bin.join("python3.12"));
    assert_eq!(std::fs::read_link(env.join("bin/python3")).unwrap(), Path::new("python"), "relative links are left alone");
    assert!(env.join("bin/python").exists(), "the link works");
    assert!(!new.join("runtimes/python/cpython-3.12-linux").exists(), "the stale shortcut is gone");
}

/// The real thing: install Python and pygame into a "Toki Hub (Tauri)" folder, rename it the way
/// version 0.2.2 does on start-up, and run the game again. Needs the internet: -- --ignored real_
#[test]
#[ignore]
fn real_installed_python_games_survive_the_rename() {
    let base = TempDir::new();
    let world = World::new();
    let build = world.dir.path().join("build");
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(build.join("main.py"), "import pygame\nprint('pygame works:', pygame.version.ver)\n").unwrap();
    write_game(&world.catalog(), "game", &game_with(vec![json!({"version": "1.0", "builds": [{"platform": "any",
        "source": {"type": "local", "path": build.to_string_lossy()},
        "launch": {"type": "python", "path": "main.py", "python": "3.12", "packages": ["pygame==2.6.1"]}}]})]));
    let run = |data: &Path| {
        let hub = Hub::new(HubOptions::new(vec![world.catalog()], data.to_path_buf()));
        if hub.installer.installed_version("game", "1.0").is_none() {
            hub.install("game", "1.0").unwrap();
            let task = wait_for_long(|| {
                let task = hub.installer.tasks().into_iter().next()?;
                (!matches!(task["status"].as_str(), Some("queued" | "downloading" | "installing"))).then_some(task)
            });
            assert_eq!(task["status"], "done", "{task}");
        }
        hub.launch("game", "1.0").unwrap();
        let event = wait_for("the game to exit", || hub.poll(0)["events"].as_array().unwrap().first().cloned());
        let log = std::fs::read_to_string(hub.launcher.log_path("game", "1.0")).unwrap();
        assert_eq!(event["code"].as_i64(), Some(0), "{log}");
        assert!(log.contains("pygame works: 2.6.1"), "{log}");
    };
    let old = base.path().join("Toki Hub (Tauri)");
    run(&old);
    let (new, log) = settle_data_dir(&base.path().join("Toki Hub"));
    assert_eq!(new, base.path().join("Toki Hub"), "{log:?}");
    assert!(!old.exists());
    run(&new);
}

fn wait_for_long<T>(mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(std::time::Instant::now() < deadline, "timed out installing");
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

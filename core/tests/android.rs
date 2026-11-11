//! Toki Hub on Android: games are apps, which a stand-in phone installs, starts and removes.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::*;
use serde_json::json;
use toki_hub_core::catalog::Catalog;
use toki_hub_core::device::Device;
use toki_hub_core::{Hub, HubError, HubOptions};

const PACKAGE: &str = "com.example.testgame.v2_0";

/// A phone: it remembers what it was asked to do, and "installs" an APK straight away.
#[derive(Default)]
struct Phone {
    apps: Mutex<Vec<String>>,
    handed: Mutex<Vec<PathBuf>>,
    launched: Mutex<Vec<String>>,
    removed: Mutex<Vec<String>>,
}

impl Device for Phone {
    fn installed(&self, package: &str) -> Option<String> {
        self.apps.lock().unwrap().iter().any(|p| p == package).then(|| "2.0".to_string())
    }
    fn install(&self, apk: &Path) -> Result<(), HubError> {
        assert!(apk.is_file(), "the APK must be there for Android's installer to read");
        assert_eq!(apk.extension().unwrap(), "apk");
        self.handed.lock().unwrap().push(apk.to_path_buf());
        self.apps.lock().unwrap().push(PACKAGE.to_string());
        Ok(())
    }
    fn launch(&self, package: &str) -> Result<(), HubError> {
        self.launched.lock().unwrap().push(package.to_string());
        Ok(())
    }
    fn uninstall(&self, package: &str) -> Result<(), HubError> {
        self.removed.lock().unwrap().push(package.to_string());
        self.apps.lock().unwrap().retain(|p| p != package);
        Ok(())
    }
}

fn game(world: &World) -> serde_json::Value {
    json!({"title": "Test Game", "versions": [
        {"version": "2.0", "builds": [
            {"platform": "any", "source": {"type": "url", "url": format!("{}/game.zip", world.web.base)},
             "launch": {"type": "python", "path": "main.py"}},
            {"platform": "android", "source": {"type": "url", "url": format!("{}/game.apk", world.web.base)},
             "launch": {"type": "android", "package": PACKAGE}}]},
        {"version": "1.0", "builds": [
            {"platform": "any", "source": {"type": "url", "url": format!("{}/game.zip", world.web.base)},
             "launch": {"type": "python", "path": "main.py"}}]}]})
}

fn phone_hub(world: &World, phone: &Arc<Phone>) -> Hub {
    let mut options = HubOptions::new(vec![world.catalog()], world.data());
    options.github_api = world.web.base.clone();
    options.platform = "android".into();
    options.device = Some(phone.clone());
    Hub::new(options)
}

#[test]
fn a_phone_installs_launches_and_removes_the_app() {
    let world = World::new();
    write_game(&world.catalog(), "test-game", &game(&world));
    world.web.file("/game.apk", b"PK fake apk".to_vec());
    let phone = Arc::new(Phone::default());
    let hub = phone_hub(&world, &phone);

    let state = hub.state();
    assert_eq!(state["android"], true);
    let versions = state["games"][0]["versions"].as_array().unwrap();
    assert_eq!(versions[0]["build"]["platform"], "android");
    assert!(versions[1]["build"].is_null(), "a computer's build is no use on a phone");
    assert!(state["installed"].as_object().unwrap().is_empty());

    let task = install_and_wait(&hub, "test-game", "2.0");
    assert_eq!(task["status"], "done", "{task}");
    assert_eq!(phone.handed.lock().unwrap().len(), 1);
    assert_eq!(hub.poll(0)["installed"]["test-game"]["2.0"]["path"], PACKAGE);

    hub.launch("test-game", "2.0").unwrap();
    assert_eq!(*phone.launched.lock().unwrap(), vec![PACKAGE.to_string()]);

    hub.uninstall("test-game", "2.0").unwrap();
    assert_eq!(*phone.removed.lock().unwrap(), vec![PACKAGE.to_string()]);
    assert!(hub.poll(0)["installed"].as_object().unwrap().is_empty());

    assert!(hub.install("test-game", "1.0").is_err(), "1.0 has no Android build");
    assert!(hub.launch("test-game", "1.0").is_err());
}

#[test]
fn a_computer_never_picks_the_android_build() {
    let world = World::new();
    write_game(&world.catalog(), "test-game", &game(&world));
    let state = world.hub().state();
    assert_eq!(state["android"], false);
    assert_eq!(state["games"][0]["versions"][0]["build"]["platform"], "any");
}

#[test]
fn the_catalog_checks_android_builds() {
    let world = World::new();
    let bad = |build: serde_json::Value| json!({"title": "Bad", "versions": [{"version": "1", "builds": [build]}]});
    let url = json!({"type": "url", "url": "https://example.com/game.apk"});
    write_game(&world.catalog(), "no-package", &bad(json!({"platform": "android", "source": url, "launch": {"type": "android"}})));
    write_game(&world.catalog(), "bad-package", &bad(json!({"platform": "android", "source": url,
        "launch": {"type": "android", "package": "not a package"}})));
    write_game(&world.catalog(), "wrong-platform", &bad(json!({"platform": "windows", "source": url,
        "launch": {"type": "android", "package": "com.example.game"}})));
    write_game(&world.catalog(), "good", &bad(json!({"platform": "android", "source": url,
        "launch": {"type": "android", "package": "com.example.game"}})));
    let catalog = Catalog::load(&[world.catalog()]);
    let problems = catalog.problems.join("\n");
    assert!(problems.contains("no-package/game.json") && problems.contains("'package' is required"), "{problems}");
    assert!(problems.contains("bad-package/game.json") && problems.contains("Android package name"), "{problems}");
    assert!(problems.contains("wrong-platform/game.json"), "{problems}");
    assert!(!problems.contains("good/"), "{problems}");
    let good = catalog.get("good").unwrap();
    assert_eq!(good.versions[0].builds[0].launch.package, "com.example.game");
}

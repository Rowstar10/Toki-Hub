//! The same checks as the Python version's tests, against the Rust core.

mod common;

use std::path::PathBuf;

use common::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use toki_hub_core::catalog::{Catalog, LaunchKind, Source};
use toki_hub_core::installer::MARKER;

fn url_source(world: &World, path: &str) -> serde_json::Value {
    json!({"type": "url", "url": format!("{}{path}", world.web.base)})
}

// ---------- Catalog ----------

#[test]
fn shipped_catalog_has_no_problems() {
    let catalog = Catalog::load(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog")]);
    assert_eq!(catalog.problems, Vec::<String>::new());
    let hu_taong = catalog.game("hu-taong").unwrap();
    assert_eq!(hu_taong.title, "Hu Taong");
    assert!(hu_taong.icon.as_ref().unwrap().is_file() && hu_taong.cover.as_ref().unwrap().is_file());
    let build = hu_taong.version("1.0").unwrap().build_for("windows").unwrap();
    assert!(matches!(build.source, Source::GithubArchive { .. }));
    assert_eq!((build.launch.kind, build.launch.path.as_str()), (LaunchKind::Python, "main.py"));
}

#[test]
fn bad_entries_are_reported_and_skipped() {
    let world = World::new();
    let catalog = world.catalog();
    std::fs::create_dir_all(catalog.join("broken")).unwrap();
    std::fs::write(catalog.join("broken/game.json"), "{\"title\": \"Broken\",, }").unwrap();
    write_game(&catalog, "untitled", &json!({"versions": []}));
    write_game(&catalog, "My Game", &json!({"title": "x"}));
    let loaded = Catalog::load(&[catalog]);
    assert!(loaded.games.is_empty());
    let problems = loaded.problems.join("\n");
    assert!(problems.contains("broken/game.json: is not valid JSON"), "{problems}");
    assert!(problems.contains("untitled/game.json: 'title' is required"), "{problems}");
    assert!(problems.contains("folder name must be lowercase"), "{problems}");
}

#[test]
fn bad_builds_are_dropped_and_typos_flagged() {
    let world = World::new();
    let mut with_typo = version_with("1.1", json!({"type": "url", "url": "https://example.com/g.zip"}));
    with_typo["builds"][0]["lauch"] = json!({});
    write_game(&world.catalog(), "game", &game_with(vec![
        version_with("1.0", json!({"type": "ftp", "url": "x"})),
        with_typo,
        version_with("1.1", json!({"type": "url", "url": "https://example.com/g.zip"})),
        version_launching("1.2", json!({"type": "url", "url": "https://x/y.zip"}), json!({"type": "executable", "path": "../evil.exe"})),
        version_with("1.3", json!({"type": "url", "url": "https://x/y.zip", "sha256": "abc"})),
        version_with("1.4", json!({"type": "github-release", "repo": "not a repo", "tag": "v1", "asset": "a.zip"})),
    ]));
    let catalog = Catalog::load(&[world.catalog()]);
    let game = catalog.game("game").unwrap();
    let names: Vec<&str> = game.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(names, ["1.4", "1.3", "1.2", "1.1", "1.0"], "newest first, the duplicate 1.1 dropped");
    assert_eq!(game.version("1.1").unwrap().builds.len(), 1, "a typo does not drop the build");
    for version in ["1.0", "1.2", "1.3", "1.4"] {
        assert!(game.version(version).unwrap().builds.is_empty(), "{version} should have lost its build");
    }
    let problems = catalog.problems.join("\n");
    for expected in ["'type' must be one of url, github-release, github-archive, local", "unknown field 'lauch'",
                     "clashes with '1.1'", "'path' must be relative", "'sha256' must be 64", "'repo' must look like"] {
        assert!(problems.contains(expected), "missing {expected:?} in:\n{problems}");
    }
}

#[test]
fn images_must_be_inside_the_game_folder() {
    let world = World::new();
    write_game(&world.catalog(), "game", &json!({"title": "Game", "icon": "../../secret.png"}));
    let catalog = Catalog::load(&[world.catalog()]);
    assert!(catalog.game("game").unwrap().icon.is_none());
    assert!(catalog.problems[0].contains("'icon' must be an image file inside"));
}

#[test]
fn platform_specific_build_wins_over_any() {
    let world = World::new();
    let mut entry = version_with("1.0", json!({"type": "url", "url": "https://x/y.zip"}));
    entry["builds"].as_array_mut().unwrap().push(json!({"platform": "windows",
        "source": {"type": "url", "url": "https://x/y.zip"}, "launch": {"type": "executable", "path": "Game.exe"}}));
    write_game(&world.catalog(), "game", &game_with(vec![entry]));
    let catalog = Catalog::load(&[world.catalog()]);
    let version = catalog.game("game").unwrap().version("1.0").unwrap();
    assert_eq!(version.build_for("windows").unwrap().launch.path, "Game.exe");
    assert_eq!(version.build_for("linux").unwrap().launch.path, "main.py");
}

#[test]
fn later_catalog_folders_add_and_replace_games() {
    let world = World::new();
    let built_in = world.dir.path().join("built-in");
    let mine = world.dir.path().join("mine");
    write_game(&built_in, "hu-taong", &json!({"title": "Hu Taong"}));
    write_game(&built_in, "other", &json!({"title": "Other"}));
    write_game(&mine, "hu-taong", &json!({"title": "Hu Taong (mine)"}));
    let catalog = Catalog::load(&[built_in, mine.clone(), world.dir.path().join("missing")]);
    assert_eq!(catalog.problems, Vec::<String>::new());
    let titles: Vec<&str> = catalog.games.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(titles, ["Hu Taong (mine)", "Other"]);
    assert_eq!(catalog.game("hu-taong").unwrap().folder, mine.join("hu-taong"));
}

// ---------- Installing ----------

#[test]
fn zip_with_a_top_folder_installs_flat() {
    let world = World::new();
    world.web.file("/game.zip", zip_bytes(GAME_FILES, Some("Game-1.0")));
    write_game(&world.catalog(), "game", &game_with(vec![version_with("1.0", url_source(&world, "/game.zip"))]));
    let hub = world.hub();
    let task = install_and_wait(&hub, "game", "1.0");
    assert_eq!(task["status"], "done", "{task}");
    let folder = hub.installer.version_dir("game", "1.0");
    assert!(folder.join("main.py").is_file() && folder.join("Resources/data.txt").is_file());
    assert!(folder.join(MARKER).is_file());
    assert!(hub.poll(0)["installed"]["game"]["1.0"]["size"].as_u64().unwrap() > 0);
    assert!(listing(&world.data().join("downloads")).is_empty(), "temp download left behind");
}

#[test]
fn checksums_are_enforced() {
    let world = World::new();
    let data = zip_bytes(GAME_FILES, None);
    let good = format!("{:x}", Sha256::digest(&data));
    world.web.file("/game.zip", data);
    let mut good_source = url_source(&world, "/game.zip");
    good_source["sha256"] = json!(good);
    let mut bad_source = url_source(&world, "/game.zip");
    bad_source["sha256"] = json!("0".repeat(64));
    write_game(&world.catalog(), "game", &game_with(vec![version_with("1.0", good_source), version_with("2.0", bad_source)]));
    let hub = world.hub();
    assert_eq!(install_and_wait(&hub, "game", "1.0")["status"], "done");
    let task = install_and_wait(&hub, "game", "2.0");
    assert_eq!(task["status"], "error");
    assert!(task["error"].as_str().unwrap().contains("does not match its sha256"));
    assert!(!hub.installer.version_dir("game", "2.0").exists());
    assert!(listing(&world.data().join("downloads")).is_empty(), "partial download left behind");
    let leftovers: Vec<String> = listing(&hub.settings.install_dir().join("game")).into_iter().filter(|n| n.starts_with('.')).collect();
    assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");
}

#[test]
fn unsafe_and_incomplete_archives_are_refused() {
    let world = World::new();
    world.web.file("/evil.zip", zip_bytes(&[("main.py", ""), ("../../escaped.txt", "gotcha")], None));
    world.web.file("/empty.zip", zip_bytes(&[("readme.txt", "no game here")], None));
    write_game(&world.catalog(), "game", &game_with(vec![
        version_with("1.0", url_source(&world, "/evil.zip")),
        version_with("2.0", url_source(&world, "/empty.zip")),
    ]));
    let hub = world.hub();
    let task = install_and_wait(&hub, "game", "1.0");
    assert!(task["error"].as_str().unwrap().contains("unsafe path"), "{task}");
    assert!(!world.dir.path().join("escaped.txt").exists() && !world.data().join("escaped.txt").exists());
    let task = install_and_wait(&hub, "game", "2.0");
    assert!(task["error"].as_str().unwrap().contains("do not contain 'main.py'"), "{task}");
}

#[test]
fn single_file_download_is_kept_as_is() {
    let world = World::new();
    world.web.file("/Game.exe", b"#!/bin/sh\necho hi\n".to_vec());
    write_game(&world.catalog(), "game", &game_with(vec![version_launching("1.0", url_source(&world, "/Game.exe"),
        json!({"type": "executable", "path": "Game.exe"}))]));
    let hub = world.hub();
    assert_eq!(install_and_wait(&hub, "game", "1.0")["status"], "done");
    let exe = hub.installer.version_dir("game", "1.0").join("Game.exe");
    assert!(exe.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert!(exe.metadata().unwrap().permissions().mode() & 0o111 != 0);
    }
}

#[test]
fn http_errors_are_explained() {
    let world = World::new();
    write_game(&world.catalog(), "game", &game_with(vec![version_with("1.0", url_source(&world, "/missing.zip"))]));
    let task = install_and_wait(&world.hub(), "game", "1.0");
    assert!(task["error"].as_str().unwrap().contains("HTTP 404"), "{task}");
}

#[test]
fn cancel_stops_the_download() {
    let world = World::new();
    world.web.route("/big.zip", || Reply::Slow(64 * 1024 * 200));
    write_game(&world.catalog(), "game", &game_with(vec![version_with("1.0", url_source(&world, "/big.zip"))]));
    let hub = world.hub();
    let task = hub.install("game", "1.0").unwrap();
    wait_for("the download to start", || (hub.installer.tasks()[0]["received"].as_u64().unwrap() > 0).then_some(()));
    assert!(hub.install("game", "1.0").is_err(), "already downloading");
    assert!(hub.cancel(task["id"].as_str().unwrap()));
    wait_for("the cancel", || (hub.installer.tasks()[0]["status"] == "cancelled").then_some(()));
    assert!(!hub.installer.version_dir("game", "1.0").exists());
    assert!(listing(&world.data().join("downloads")).is_empty(), "partial download left behind");
}

#[test]
fn reinstall_uninstall_and_side_by_side_versions() {
    let world = World::new();
    world.web.file("/game.zip", zip_bytes(GAME_FILES, None));
    let source = url_source(&world, "/game.zip");
    write_game(&world.catalog(), "game", &game_with(vec![version_with("1.0", source.clone()), version_with("2.0-beta", source)]));
    let hub = world.hub();
    install_and_wait(&hub, "game", "1.0");
    install_and_wait(&hub, "game", "2.0-beta");
    let installed = hub.poll(0)["installed"]["game"].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(installed, ["1.0", "2.0-beta"]);
    let folder = hub.installer.version_dir("game", "1.0");
    std::fs::write(folder.join("leftover.txt"), "from the old install").unwrap();
    assert_eq!(install_and_wait(&hub, "game", "1.0")["status"], "done");
    assert!(!folder.join("leftover.txt").exists(), "reinstall should replace the old files");
    hub.uninstall("game", "1.0").unwrap();
    hub.uninstall("game", "2.0-beta").unwrap();
    assert!(!folder.parent().unwrap().exists(), "empty game folder should be tidied away");
    assert!(hub.uninstall("game", "1.0").is_err());
}

#[test]
fn no_build_for_this_platform() {
    let world = World::new();
    let mut entry = version_with("1.0", json!({"type": "url", "url": "https://x/y.zip"}));
    entry["builds"][0]["platform"] = json!("macos");
    write_game(&world.catalog(), "game", &game_with(vec![entry]));
    let error = world.hub().install("game", "1.0").unwrap_err();
    assert!(error.0.contains("no build for windows"), "{error}");
}

#[test]
fn local_folders_and_zips_are_copied() {
    let world = World::new();
    let build = world.dir.path().join("my-build");
    std::fs::create_dir_all(build.join(".git")).unwrap();
    std::fs::write(build.join("main.py"), "print('hi')").unwrap();
    let folder = write_game(&world.catalog(), "game", &game_with(vec![
        version_with("1.0", json!({"type": "local", "path": build.to_string_lossy()})),
        version_with("2.0", json!({"type": "local", "path": "builds/2.0.zip"})),
        version_with("3.0", json!({"type": "local", "path": world.dir.path().join("nowhere").to_string_lossy()})),
    ]));
    std::fs::create_dir_all(folder.join("builds")).unwrap();
    std::fs::write(folder.join("builds/2.0.zip"), zip_bytes(GAME_FILES, None)).unwrap();
    let hub = world.hub();
    assert_eq!(install_and_wait(&hub, "game", "1.0")["status"], "done");
    let installed = hub.installer.version_dir("game", "1.0");
    assert!(installed.join("main.py").is_file() && !installed.join(".git").exists());
    assert_eq!(install_and_wait(&hub, "game", "2.0")["status"], "done");
    let task = install_and_wait(&hub, "game", "3.0");
    assert!(task["error"].as_str().unwrap().contains("not on this computer"), "{task}");
}

#[test]
fn private_github_release_keeps_the_token_off_redirects() {
    let world = World::new();
    world.web.json("/repos/Owner/Game/releases/tags/v1.0",
        json!({"assets": [{"name": "Game-windows.zip", "url": format!("{}/assets/7", world.web.base)}]}));
    world.web.redirect("/assets/7", "/storage/signed.zip");
    world.web.file("/storage/signed.zip", zip_bytes(GAME_FILES, Some("Game")));
    write_game(&world.catalog(), "game", &game_with(vec![version_with("1.0",
        json!({"type": "github-release", "repo": "Owner/Game", "tag": "v1.0", "asset": "Game-windows.zip"}))]));
    let hub = world.hub();
    hub.save_settings(&json!({"github_token": "secret-token"})).unwrap();
    let task = install_and_wait(&hub, "game", "1.0");
    assert_eq!(task["status"], "done", "{task}");
    let api = &world.web.headers_for("/repos/Owner/Game/releases/tags/v1.0")[0];
    assert_eq!(api.get("authorization").map(String::as_str), Some("Bearer secret-token"));
    let asset = &world.web.headers_for("/assets/7")[0];
    assert_eq!(asset.get("authorization").map(String::as_str), Some("Bearer secret-token"));
    assert_eq!(asset.get("accept").map(String::as_str), Some("application/octet-stream"));
    let storage = &world.web.headers_for("/storage/signed.zip")[0];
    assert!(!storage.contains_key("authorization"), "token must not be sent to the redirect target");
}

#[test]
fn github_problems_are_explained() {
    let world = World::new();
    world.web.json("/repos/Owner/Game/releases/tags/v1.0", json!({"assets": [{"name": "Other.zip", "url": "x"}]}));
    write_game(&world.catalog(), "game", &game_with(vec![
        version_with("1.0", json!({"type": "github-release", "repo": "Owner/Game", "tag": "v1.0", "asset": "Game.zip"})),
        version_with("2.0", json!({"type": "github-release", "repo": "Owner/Private", "tag": "v2.0", "asset": "Game.zip"})),
    ]));
    let hub = world.hub();
    let task = install_and_wait(&hub, "game", "1.0");
    assert!(task["error"].as_str().unwrap().contains("no file named 'Game.zip' (it has: Other.zip)"), "{task}");
    let task = install_and_wait(&hub, "game", "2.0");
    assert!(task["error"].as_str().unwrap().contains("If the repository is private, add a GitHub token"), "{task}");
}

#[test]
fn github_source_archive() {
    let world = World::new();
    let reference = "db6da17c8519a61731f6cffb39da2e2cd2730935";
    world.web.redirect(&format!("/repos/Owner/Game/zipball/{reference}"), "/codeload/archive.zip");
    world.web.file("/codeload/archive.zip", zip_bytes(GAME_FILES, Some("Owner-Game-db6da17")));
    write_game(&world.catalog(), "game", &game_with(vec![version_with("1.0",
        json!({"type": "github-archive", "repo": "Owner/Game", "ref": reference}))]));
    let hub = world.hub();
    assert_eq!(install_and_wait(&hub, "game", "1.0")["status"], "done");
    assert!(hub.installer.version_dir("game", "1.0").join("main.py").is_file());
    let api = &world.web.headers_for(&format!("/repos/Owner/Game/zipball/{reference}"))[0];
    assert!(!api.contains_key("authorization"), "no token configured, so none should be sent");
}

// ---------- Launching ----------

fn install_local(world: &World, files: &[(&str, &str)], launch: serde_json::Value) -> toki_hub_core::Hub {
    let build = world.dir.path().join("build");
    std::fs::create_dir_all(&build).unwrap();
    for (name, content) in files {
        std::fs::write(build.join(name), content).unwrap();
    }
    write_game(&world.catalog(), "game", &game_with(vec![version_launching("1.0",
        json!({"type": "local", "path": build.to_string_lossy()}), launch)]));
    let hub = world.hub();
    assert_eq!(install_and_wait(&hub, "game", "1.0")["status"], "done");
    hub
}

fn wait_exit(hub: &toki_hub_core::Hub) -> serde_json::Value {
    wait_for("the game to exit", || hub.poll(0)["events"].as_array().unwrap().first().cloned())
}

#[test]
fn python_game_runs_in_its_folder_and_is_logged() {
    if python().is_none() {
        return;
    }
    let world = World::new();
    let hub = install_local(&world, &[("main.py", "import os\nprint('hello from', os.path.basename(os.getcwd()))\n")],
                            json!({"type": "python", "path": "main.py"}));
    assert_eq!(hub.launch("game", "1.0").unwrap()["tracked"], true);
    let event = wait_exit(&hub);
    assert_eq!((event["code"].as_i64(), &event["stopped"], &event["log_tail"]), (Some(0), &json!(false), &json!("")));
    let log = std::fs::read_to_string(hub.launcher.log_path("game", "1.0")).unwrap();
    assert!(log.contains("hello from 1.0"), "{log}");
    assert!(hub.poll(0)["running"].as_array().unwrap().is_empty());
}

#[test]
fn crash_reports_the_end_of_the_log() {
    if python().is_none() {
        return;
    }
    let world = World::new();
    let hub = install_local(&world, &[("main.py", "import sys\nprint('No module named pygame', file=sys.stderr)\nsys.exit(3)\n")],
                            json!({"type": "python", "path": "main.py"}));
    hub.launch("game", "1.0").unwrap();
    let event = wait_exit(&hub);
    assert_eq!(event["code"].as_i64(), Some(3));
    assert!(event["log_tail"].as_str().unwrap().contains("No module named pygame"));
    let id = event["id"].as_u64().unwrap();
    assert!(hub.poll(id)["events"].as_array().unwrap().is_empty(), "events are only delivered once per cursor");
}

#[test]
fn one_instance_per_version_and_stop() {
    if python().is_none() {
        return;
    }
    let world = World::new();
    let hub = install_local(&world, &[("main.py", "import time\ntime.sleep(30)\n")], json!({"type": "python", "path": "main.py"}));
    hub.launch("game", "1.0").unwrap();
    assert_eq!(hub.poll(0)["running"].as_array().unwrap().len(), 1);
    assert!(hub.launch("game", "1.0").is_err());
    assert!(hub.uninstall("game", "1.0").is_err());
    assert!(hub.install("game", "1.0").is_err());
    hub.stop("game", "1.0").unwrap();
    let event = wait_exit(&hub);
    assert_eq!((&event["stopped"], &event["log_tail"]), (&json!(true), &json!("")), "a stop the player asked for is not a crash");
}

#[cfg(unix)]
#[test]
fn executable_game() {
    let world = World::new();
    let hub = install_local(&world, &[("run.sh", "#!/bin/sh\necho native game\n")], json!({"type": "executable", "path": "run.sh"}));
    hub.launch("game", "1.0").unwrap();
    assert_eq!(wait_exit(&hub)["code"].as_i64(), Some(0));
    assert!(std::fs::read_to_string(hub.launcher.log_path("game", "1.0")).unwrap().contains("native game"));
}

#[test]
fn missing_python_is_explained() {
    let world = World::new();
    let hub = install_local(&world, &[("main.py", "")], json!({"type": "python", "path": "main.py"}));
    hub.settings.update(&[("python", world.dir.path().join("no-such-python").to_string_lossy().into_owned())]).unwrap();
    let error = hub.launch("game", "1.0").unwrap_err();
    assert!(error.0.contains("Set the Python interpreter in Settings"), "{error}");
}

// ---------- Settings and images ----------

#[test]
fn settings_never_expose_the_token() {
    let world = World::new();
    let hub = world.hub();
    let settings = hub.save_settings(&json!({"github_token": "ghp_secret"})).unwrap();
    assert_eq!(settings["has_token"], true);
    assert!(!settings.to_string().contains("ghp_secret") && !hub.state().to_string().contains("ghp_secret"));
    assert!(hub.save_settings(&json!({"install_dir": "relative/path"})).is_err());
    assert_eq!(hub.clear_token().unwrap()["has_token"], false);
}

#[test]
fn only_catalog_images_are_served() {
    let world = World::new();
    let folder = write_game(&world.catalog(), "game", &json!({"title": "Game", "icon": "icon.png"}));
    std::fs::write(folder.join("icon.png"), b"\x89PNG").unwrap();
    std::fs::write(world.catalog().join("secret.png"), b"secret").unwrap();
    let hub = world.hub();
    assert_eq!(hub.state()["games"][0]["icon"], "game/icon.png");
    assert_eq!(hub.image("game", "icon.png").unwrap().1, "image/png");
    assert!(hub.image("game", "game.json").is_none());
    assert!(hub.image("game", "../secret.png").is_none());
    assert!(hub.image("nope", "icon.png").is_none());
}

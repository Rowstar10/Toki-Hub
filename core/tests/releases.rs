//! Games published as GitHub releases (core/src/releases.rs): listing their versions signed out,
//! installing them, and what happens offline, at GitHub's rate limit, and with a broken download.

mod common;

use common::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use toki_hub_core::hub::{Hub, HubOptions};

const LIST: &str = "/repos/Owner/Game-Releases/releases?per_page=100";

fn game_json() -> Value {
    json!({"title": "Game", "releases": {"repo": "Owner/Game-Releases", "builds": [
        {"platform": "windows", "asset": "Game-{version}-windows.zip",
         "launch": {"type": "executable", "path": "Game.exe"}},
        {"platform": "android", "asset": "Game-{version}-android.apk",
         "launch": {"type": "android", "package": "com.example.game.v{version_}"}, "note": "Touch controls ({version})"}
    ]}})
}

fn build(text: &str) -> Vec<u8> {
    zip_bytes(&[("Game.exe", text), ("data/level.txt", "level")], Some("Game"))
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn release(tag: &str, assets: &[(&str, &[u8])]) -> Value {
    json!({
        "tag_name": tag, "draft": false, "prerelease": false, "published_at": "2026-11-20T10:00:00Z",
        "body": format!("What {tag} is about.\n\n### Added\n- One thing\n- Another\n\n---\n\nDownload it below."),
        "assets": assets.iter().map(|(name, bytes)| json!({
            "name": name, "size": bytes.len(), "digest": format!("sha256:{}", sha256(bytes)),
        })).collect::<Vec<_>>(),
    })
}

fn serve(world: &World, version: &str, bytes: Vec<u8>) {
    world.web.file(&format!("/Owner/Game-Releases/releases/download/v{version}/Game-{version}-windows.zip"), bytes);
}

/// A hub that checks for releases only when told to (as reload and start-up do for real).
fn hub(world: &World, api: &str) -> Hub {
    let mut options = HubOptions::new(vec![world.catalog()], world.data());
    options.github_api = api.to_string();
    options.github_web = world.web.base.clone();
    options.platform = "windows".into();
    options.check_releases = false;
    Hub::new(options)
}

fn versions(state: &Value) -> Vec<String> {
    state["games"][0]["versions"].as_array().unwrap().iter().map(|v| v["version"].as_str().unwrap().to_string()).collect()
}

#[test]
fn versions_come_from_releases_and_install_without_a_token() {
    let world = World::new();
    write_game(&world.catalog(), "game", &game_json());
    let (v11, v10) = (build("1.1"), build("1.0"));
    let mut draft = release("v2.0", &[("Game-2.0-windows.zip", b"x")]);
    draft["draft"] = json!(true);
    world.web.json(LIST, json!([
        release("v1.1", &[("Game-1.1-windows.zip", &v11), ("Game-1.1-android.apk", b"apk")]),
        draft,
        release("nightly", &[("Game-nightly-windows.zip", b"x")]),     // not a version tag
        release("v0.9", &[("Notes.txt", b"x")]),                        // none of the game's files
        release("v1.0", &[("Game-1.0-windows.zip", &v10)]),
    ]));
    serve(&world, "1.1", v11);
    serve(&world, "1.0", v10);
    let hub = hub(&world, &world.web.base);
    assert!(versions(&hub.state()).is_empty(), "nothing known before the first check");

    let state = hub.check_releases();
    assert_eq!(versions(&state), ["1.1", "1.0"]);
    let newest = &state["games"][0]["versions"][0];
    assert_eq!(newest["build"]["platform"], "windows");
    assert_eq!(newest["date"], "2026-11-20");
    assert_eq!(newest["notes"], "What v1.1 is about.", "the first paragraph is the summary");
    assert_eq!(newest["platforms"], json!(["android", "windows"]));
    assert_eq!(state["releases"]["game"]["from"], "online");
    let notes = hub.changelog("game").unwrap();
    let full = notes["versions"]["1.1"].as_str().unwrap();
    assert!(full.contains("### Added") && !full.contains("Download it below"), "{full}");

    // Two versions side by side, from the public download links, signed out
    assert_eq!(install_and_wait(&hub, "game", "1.1")["status"], "done");
    assert_eq!(install_and_wait(&hub, "game", "1.0")["status"], "done");
    for version in ["1.1", "1.0"] {
        let folder = hub.installer.version_dir("game", version);
        assert_eq!(std::fs::read_to_string(folder.join("Game.exe")).unwrap(), version, "the zip's top folder is dropped");
    }
    for path in [LIST, "/Owner/Game-Releases/releases/download/v1.1/Game-1.1-windows.zip"] {
        let requests = world.web.headers_for(path);
        assert!(!requests.is_empty() && requests.iter().all(|h| !h.contains_key("authorization")), "{path}: {requests:?}");
    }
}

#[test]
fn the_android_build_comes_from_the_same_release() {
    let world = World::new();
    write_game(&world.catalog(), "game", &game_json());
    world.web.json(LIST, json!([release("v1.1", &[("Game-1.1-windows.zip", b"zip"), ("Game-1.1-android.apk", b"apk")])]));
    let catalog_hub = hub(&world, &world.web.base);
    catalog_hub.check_releases();
    let catalog = catalog_hub.catalog();
    let build = catalog.game("game").unwrap().version("1.1").unwrap().build_for("android").unwrap().clone();
    assert_eq!(build.launch.package, "com.example.game.v1_1");
    assert_eq!(build.note, "Touch controls (1.1)");
}

#[test]
fn the_list_is_kept_for_no_internet_and_the_rate_limit() {
    let world = World::new();
    write_game(&world.catalog(), "game", &game_json());
    let list = json!([release("v1.0", &[("Game-1.0-windows.zip", b"zip")])]).to_string().into_bytes();
    let first = list.clone();
    world.web.route(LIST, move || Reply::Body(200, vec![("ETag".into(), "\"abc\"".into())], first.clone()));
    assert_eq!(versions(&hub(&world, &world.web.base).check_releases()), ["1.0"]);

    // Next start: the saved list shows straight away, before GitHub has answered
    let reset = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 600;
    world.web.route(LIST, move || Reply::Body(403, vec![
        ("X-RateLimit-Remaining".into(), "0".into()), ("X-RateLimit-Reset".into(), reset.to_string()),
    ], b"rate limit exceeded".to_vec()));
    let hub2 = hub(&world, &world.web.base);
    let state = hub2.state();
    assert_eq!((versions(&state), state["releases"]["game"]["from"].clone()), (vec!["1.0".to_string()], json!("saved")));
    let state = hub2.check_releases();
    assert_eq!(versions(&state), ["1.0"], "the limit doesn't lose the versions");
    let problem = state["releases"]["game"]["problem"].as_str().unwrap();
    assert!(problem.contains("limit") && problem.contains("10 minutes"), "{problem}");
    let last = world.web.headers_for(LIST).pop().unwrap();
    assert_eq!(last.get("if-none-match").map(String::as_str), Some("\"abc\""), "asked with the last ETag");

    // "Nothing new" (which GitHub doesn't count against the limit)
    world.web.route(LIST, || Reply::Body(304, vec![], vec![]));
    let state = hub2.check_releases();
    assert_eq!((versions(&state), state["releases"]["game"]["from"].clone()), (vec!["1.0".to_string()], json!("online")));
    assert_eq!(state["releases"]["game"]["problem"], "");

    // No internet at all
    let offline = hub(&world, "http://127.0.0.1:9");
    let state = offline.check_releases();
    assert_eq!(versions(&state), ["1.0"]);
    assert!(state["releases"]["game"]["problem"].as_str().unwrap().contains("internet"), "{state}");
}

#[test]
fn a_broken_download_leaves_the_installed_version_working() {
    let world = World::new();
    write_game(&world.catalog(), "game", &game_json());
    let good = build("good");
    let mut no_digest = release("v1.1", &[("Game-1.1-windows.zip", b"x")]);
    no_digest["assets"][0]["digest"] = Value::Null;      // (GitHub's older releases have none)
    world.web.json(LIST, json!([no_digest, release("v1.0", &[("Game-1.0-windows.zip", &good)])]));
    serve(&world, "1.0", good);
    let hub = hub(&world, &world.web.base);
    hub.check_releases();
    assert_eq!(install_and_wait(&hub, "game", "1.0")["status"], "done");
    let folder = hub.installer.version_dir("game", "1.0");

    // Reinstalling, the file arrives damaged: its sha256 doesn't match what GitHub published
    serve(&world, "1.0", build("damaged"));
    let task = install_and_wait(&hub, "game", "1.0");
    assert_eq!(task["status"], "error", "{task}");
    assert!(task["error"].as_str().unwrap().contains("sha256"), "{task}");
    assert_eq!(std::fs::read_to_string(folder.join("Game.exe")).unwrap(), "good", "the installed copy is untouched");

    // A cut-off zip with no sha256 to check: it can't be unpacked, and nothing changes
    let mut cut = build("cut");
    cut.truncate(cut.len() / 2);
    serve(&world, "1.1", cut);
    assert_eq!(install_and_wait(&hub, "game", "1.1")["status"], "error");
    assert!(!hub.installer.version_dir("game", "1.1").exists());
    assert_eq!(listing(folder.parent().unwrap()), ["1.0"], "no half-made folders are left");
    assert_eq!(hub.installer.installed_version("game", "1.0").unwrap()["version"], "1.0");
}

#[test]
fn a_reinstall_cut_off_midway_is_put_back_at_the_next_start() {
    let world = World::new();
    write_game(&world.catalog(), "game", &game_json());
    let zip = build("1.0");
    world.web.json(LIST, json!([release("v1.0", &[("Game-1.0-windows.zip", &zip)])]));
    serve(&world, "1.0", zip);
    let hub1 = hub(&world, &world.web.base);
    hub1.check_releases();
    assert_eq!(install_and_wait(&hub1, "game", "1.0")["status"], "done");
    assert_eq!(install_and_wait(&hub1, "game", "1.0")["status"], "done", "a reinstall over it works");
    let folder = hub1.installer.version_dir("game", "1.0");
    let games = folder.parent().unwrap().to_path_buf();
    assert_eq!(listing(&games), ["1.0"], "the old copy was deleted once the new one was in");

    // The hub stopped between moving the old copy aside and putting the new one in
    std::fs::rename(&folder, games.join(".old-1.0-t7")).unwrap();
    drop(hub1);
    let hub2 = hub(&world, &world.web.base);
    assert_eq!(listing(&games), ["1.0"], "put back");
    assert!(hub2.installer.installed_version("game", "1.0").is_some());

    // ...or after putting the new one in: the old copy just goes
    std::fs::create_dir_all(games.join(".old-1.0-t8")).unwrap();
    drop(hub2);
    let _hub3 = hub(&world, &world.web.base);
    assert_eq!(listing(&games), ["1.0"]);
}

#[test]
fn listed_versions_keep_their_summary_and_get_the_release_notes() {
    let world = World::new();
    let mut game = game_json();
    game["versions"] = json!([{"version": "1.0", "date": "2026-01-01", "notes": "A hand-written summary"}]);
    write_game(&world.catalog(), "game", &game);
    world.web.json(LIST, json!([release("v1.0", &[("Game-1.0-windows.zip", b"zip")])]));
    let hub = hub(&world, &world.web.base);
    assert_eq!(hub.state()["games"][0]["versions"][0]["build"], Value::Null, "listed, with no build until its release is known");
    let state = hub.check_releases();
    let version = &state["games"][0]["versions"][0];
    assert_eq!((version["notes"].as_str(), version["date"].as_str()), (Some("A hand-written summary"), Some("2026-01-01")));
    assert_eq!(version["build"]["platform"], "windows");
    assert!(hub.changelog("game").unwrap()["versions"]["1.0"].as_str().unwrap().contains("One thing"));
}

#[test]
fn the_catalog_checks_releases() {
    let world = World::new();
    let bad = |releases: Value| json!({"title": "Bad", "releases": releases});
    write_game(&world.catalog(), "no-repo", &bad(json!({"builds": []})));
    write_game(&world.catalog(), "bad-tag", &bad(json!({"repo": "a/b", "tag": "release", "builds": []})));
    write_game(&world.catalog(), "android-exe", &bad(json!({"repo": "a/b", "builds": [
        {"platform": "android", "asset": "x.apk", "launch": {"type": "executable", "path": "x.exe"}}]})));
    write_game(&world.catalog(), "asset-path", &bad(json!({"repo": "a/b", "builds": [
        {"platform": "windows", "asset": "dir/x.zip", "launch": {"type": "executable", "path": "x.exe"}}]})));
    let problems = hub(&world, &world.web.base).state()["problems"].to_string();
    for expected in ["'repo' is required", "'tag' must contain {version}", "an \\\"android\\\" build needs", "'asset' must be a file name"] {
        assert!(problems.contains(expected), "{expected} not in {problems}");
    }
}

//! Toki Hub: a window showing `../ui`, whose buttons call the commands below,
//! which call the hub's logic in `../core`.
//!
//! The catalog is built into the exe. At start-up it is unpacked to `<data folder>/cache/catalog`
//! and read together with `<data folder>/catalog`, where games can be added without rebuilding.
//!
//! Updates: the hub checks this repository's `tauri-v*` releases (see core/src/updates.rs). To
//! install one it swaps the exe and starts the new one with `--updated`, which waits for the old
//! copy to close and then deletes it.
//!
//! Command line (for testing): `Toki Hub.exe [--data-dir <folder>] [check]`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod log;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use include_dir::{include_dir, Dir};
use serde_json::Value;
use tauri::http::{header::CONTENT_TYPE, Request, Response};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use toki_hub_core::catalog::Catalog;
use toki_hub_core::sources::unquote;
use toki_hub_core::updates::{remove_old_exe, swap_exe, UpdateSource};
use toki_hub_core::{paths, Hub, HubError, HubOptions};

/// Where updates come from.
const UPDATE_REPO: &str = "Rowstar10/Toki-Hub";
const UPDATE_TAG_PREFIX: &str = "tauri-v";
/// The release file for this computer.
const UPDATE_ASSET_SUFFIX: &str = if cfg!(windows) { ".exe" } else if cfg!(target_os = "macos") { "-macos" } else { "-linux-x86_64" };

/// This exe's path and the arguments it was started with, to start it again after an update.
static RELAUNCH: OnceLock<(PathBuf, Vec<String>)> = OnceLock::new();

static BUILT_IN_CATALOG: Dir = include_dir!("$CARGO_MANIFEST_DIR/../catalog");

type HubState<'a> = State<'a, Arc<Hub>>;
type Answer<T> = Result<T, String>;

/// Run hub work off the UI thread.
async fn work<T: Send + 'static>(hub: &Arc<Hub>, job: impl FnOnce(&Hub) -> Result<T, HubError> + Send + 'static) -> Answer<T> {
    let hub = hub.clone();
    match tauri::async_runtime::spawn_blocking(move || job(&hub)).await {
        Ok(result) => result.map_err(|error| error.0),
        Err(error) => Err(format!("Unexpected error: {error}")),
    }
}

// -- Commands the UI can call (ui/js/hub.js lists their argument names) --

#[tauri::command]
async fn state(hub: HubState<'_>) -> Answer<Value> {
    static FIRST: AtomicBool = AtomicBool::new(true);
    let state = work(&hub, |hub| Ok(hub.state())).await?;
    if FIRST.swap(false, Ordering::Relaxed) {
        log::line(&format!("UI loaded: {} game(s)", state["games"].as_array().map_or(0, Vec::len)));
    }
    Ok(state)
}

#[tauri::command]
async fn poll(hub: HubState<'_>, after_event: u64) -> Answer<Value> {
    work(&hub, move |hub| Ok(hub.poll(after_event))).await
}

#[tauri::command]
async fn reload(hub: HubState<'_>) -> Answer<Value> {
    work(&hub, |hub| Ok(hub.reload())).await
}

#[tauri::command]
async fn install(hub: HubState<'_>, game_id: String, version: String) -> Answer<Value> {
    work(&hub, move |hub| hub.install(&game_id, &version)).await
}

#[tauri::command]
async fn cancel(hub: HubState<'_>, task_id: String) -> Answer<bool> {
    work(&hub, move |hub| Ok(hub.cancel(&task_id))).await
}

#[tauri::command]
async fn uninstall(hub: HubState<'_>, game_id: String, version: String) -> Answer<()> {
    work(&hub, move |hub| hub.uninstall(&game_id, &version)).await
}

#[tauri::command]
async fn launch(hub: HubState<'_>, game_id: String, version: String) -> Answer<Value> {
    work(&hub, move |hub| hub.launch(&game_id, &version)).await
}

#[tauri::command]
async fn stop(hub: HubState<'_>, game_id: String, version: String) -> Answer<()> {
    work(&hub, move |hub| hub.stop(&game_id, &version)).await
}

#[tauri::command]
async fn open_folder(hub: HubState<'_>, game_id: String, version: String) -> Answer<()> {
    work(&hub, move |hub| hub.open_folder(&game_id, &version)).await
}

#[tauri::command]
async fn open_log(hub: HubState<'_>, game_id: String, version: String) -> Answer<()> {
    work(&hub, move |hub| hub.open_log(&game_id, &version)).await
}

#[tauri::command]
async fn open_place(hub: HubState<'_>, which: String) -> Answer<()> {
    work(&hub, move |hub| hub.open_place(&which)).await
}

#[tauri::command]
async fn save_settings(hub: HubState<'_>, changes: Value) -> Answer<Value> {
    work(&hub, move |hub| hub.save_settings(&changes)).await
}

#[tauri::command]
async fn clear_token(hub: HubState<'_>) -> Answer<Value> {
    work(&hub, |hub| hub.clear_token()).await
}

#[tauri::command]
async fn check_for_updates(hub: HubState<'_>) -> Answer<Value> {
    work(&hub, |hub| Ok(hub.check_for_updates())).await
}

#[tauri::command]
async fn download_update(hub: HubState<'_>) -> Answer<Value> {
    work(&hub, |hub| hub.download_update()).await
}

/// Put the downloaded update in place of this exe, start it, and close this copy.
#[tauri::command]
async fn apply_update(app: AppHandle, hub: HubState<'_>) -> Answer<()> {
    let Some(file) = hub.update_file() else { return Err("The update has not finished downloading.".into()) };
    let Some((exe, args)) = RELAUNCH.get().cloned() else { return Err("Toki Hub does not know where it is.".into()) };
    let swap_target = exe.clone();
    tauri::async_runtime::spawn_blocking(move || swap_exe(&swap_target, &file))
        .await
        .map_err(|error| format!("Unexpected error: {error}"))?
        .map_err(|error| error.0)?;
    log::line("Update put in place; restarting");
    std::process::Command::new(&exe)
        .args(&args)
        .arg("--updated")
        .spawn()
        .map_err(|error| format!("The update is in place, but Toki Hub could not restart ({error}). Please start it again."))?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Answer<Option<String>> {
    let chosen = tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(|error| format!("Could not open the folder picker: {error}"))?;
    Ok(chosen.and_then(|path| path.into_path().ok()).map(|path| path.display().to_string()))
}

/// Serves `tokicatalog://localhost/<game>/<image>`: catalog images, and nothing else.
fn catalog_image(app: &AppHandle, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = unquote(request.uri().path().trim_start_matches('/'));
    let image = app
        .try_state::<Arc<Hub>>()
        .and_then(|hub| path.split_once('/').and_then(|(game, file)| hub.image(game, file)));
    match image {
        Some((bytes, content_type)) => Response::builder().header(CONTENT_TYPE, content_type).body(bytes),
        None => Response::builder().status(404).body(Vec::new()),
    }
    .unwrap_or_else(|_| Response::new(Vec::new()))
}

/// Unpack the built-in catalog, then list it with the player's own catalog folder.
fn catalog_dirs(data_dir: &Path) -> Vec<PathBuf> {
    let built_in = data_dir.join("cache").join("catalog");
    let _ = std::fs::remove_dir_all(&built_in);
    if let Err(error) = std::fs::create_dir_all(&built_in).and_then(|_| BUILT_IN_CATALOG.extract(&built_in)) {
        log::line(&format!("Could not unpack the built-in catalog: {error}"));
    }
    vec![built_in, data_dir.join("catalog")]
}

fn main() {
    let original_args: Vec<String> = std::env::args().skip(1).filter(|arg| arg != "--updated").collect();
    let just_updated = std::env::args().any(|arg| arg == "--updated");
    let mut args = original_args.iter();
    let mut data_dir = None;
    let mut check = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => data_dir = args.next().map(PathBuf::from),
            "check" => check = true,
            _ => {}
        }
    }
    let data_dir = data_dir.unwrap_or_else(paths::default_data_dir);
    log::start(&data_dir.join("logs").join("hub.log"));
    let catalog_dirs = catalog_dirs(&data_dir);

    if check {
        let catalog = Catalog::load(&catalog_dirs);
        for problem in &catalog.problems {
            log::line(&format!("Catalog problem: {problem}"));
            eprintln!("{problem}");
        }
        log::line(&format!("Catalog check: {} game(s), {} problem(s)", catalog.games.len(), catalog.problems.len()));
        std::process::exit(if catalog.problems.is_empty() { 0 } else { 1 });
    }

    log::line(&format!("Toki Hub {} starting (Tauri; data: {})", env!("CARGO_PKG_VERSION"), data_dir.display()));
    if let Ok(exe) = std::env::current_exe() {
        if just_updated {
            // Wait for the old copy to close, so the two never share the window's browser engine.
            let gone = remove_old_exe(&exe, std::time::Duration::from_secs(15));
            log::line(if gone { "Finished updating" } else { "The old version is still closing" });
        } else {
            let exe = exe.clone();
            std::thread::spawn(move || remove_old_exe(&exe, std::time::Duration::from_secs(30)));
        }
        let _ = RELAUNCH.set((exe, original_args.clone()));
    }
    let app_handle: Arc<OnceLock<AppHandle>> = Arc::default();
    let opener_handle = app_handle.clone();
    let mut options = HubOptions::new(catalog_dirs, data_dir);
    options.hub_version = env!("CARGO_PKG_VERSION").to_string();
    options.just_updated = just_updated;
    options.updates = Some(UpdateSource {
        repo: UPDATE_REPO.into(),
        tag_prefix: UPDATE_TAG_PREFIX.into(),
        asset_suffix: UPDATE_ASSET_SUFFIX.into(),
    });
    if cfg!(debug_assertions) {
        // Test builds only: point the hub at a stand-in for GitHub.
        if let Ok(api) = std::env::var("TOKIHUB_GITHUB_API") {
            options.github_api = api;
        }
    }
    options.opener = Arc::new(move |path: &Path| {
        let Some(app) = opener_handle.get() else { return Err(HubError("Toki Hub is still starting.".into())) };
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(|error| HubError(format!("Could not open {}: {error}", path.display())))
    });
    let hub = Arc::new(Hub::new(options));

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(hub)
        .register_uri_scheme_protocol("tokicatalog", |context, request| catalog_image(context.app_handle(), &request))
        .on_page_load(|_, payload| {
            if matches!(payload.event(), PageLoadEvent::Finished) {
                log::line("Window shown");
            }
        })
        .setup(move |app| {
            let _ = app_handle.set(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            state, poll, reload, install, cancel, uninstall, launch, stop,
            open_folder, open_log, open_place, save_settings, clear_token, pick_folder,
            check_for_updates, download_update, apply_update
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        log::line(&format!("Toki Hub could not start: {error}"));
        std::process::exit(1);
    }
}

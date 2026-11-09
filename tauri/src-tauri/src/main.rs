//! Toki Hub, the Tauri version: a window showing `../ui`, whose buttons call the commands below,
//! which call the hub's logic in `../core`.
//!
//! The catalog is built into the exe. At start-up it is unpacked to `<data folder>/cache/catalog`
//! and read together with `<data folder>/catalog`, where games can be added without rebuilding.
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
use toki_hub_core::{paths, Hub, HubError, HubOptions};

static BUILT_IN_CATALOG: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../catalog");

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
    let mut args = std::env::args().skip(1);
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
    let app_handle: Arc<OnceLock<AppHandle>> = Arc::default();
    let opener_handle = app_handle.clone();
    let mut options = HubOptions::new(catalog_dirs, data_dir);
    options.hub_version = env!("CARGO_PKG_VERSION").to_string();
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
            open_folder, open_log, open_place, save_settings, clear_token, pick_folder
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        log::line(&format!("Toki Hub could not start: {error}"));
        std::process::exit(1);
    }
}

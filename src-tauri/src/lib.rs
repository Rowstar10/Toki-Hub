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
//!
//! On Android (see `mobile_run`), games are apps: the hub downloads their APKs and asks Android to
//! install, start and remove them, through the Android plugin in `../plugins/device`. Its own
//! updates are APKs too.

mod log;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(desktop)]
use std::sync::OnceLock;

use include_dir::{include_dir, Dir};
use serde_json::Value;
use tauri::http::{header::CONTENT_TYPE, Request, Response};
#[cfg(desktop)]
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, State};
#[cfg(desktop)]
use tauri_plugin_dialog::DialogExt;
#[cfg(desktop)]
use tauri_plugin_opener::OpenerExt;
#[cfg(desktop)]
use toki_hub_core::catalog::Catalog;
use toki_hub_core::sources::unquote;
#[cfg(desktop)]
use toki_hub_core::updates::{remove_old_exe, swap_exe};
use toki_hub_core::updates::UpdateSource;
#[cfg(desktop)]
use toki_hub_core::{migrate, paths};
use toki_hub_core::{Hub, HubError, HubOptions};

/// Where updates come from.
const UPDATE_REPO: &str = "Rowstar10/Toki-Hub";
const UPDATE_TAG_PREFIX: &str = "tauri-v";
/// The release file for this computer (or phone).
const UPDATE_ASSET_SUFFIX: &str = if cfg!(target_os = "android") {
    "-android.apk"
} else if cfg!(windows) {
    ".exe"
} else if cfg!(target_os = "macos") {
    "-macos"
} else {
    "-linux-x86_64"
};

/// This exe's path and the arguments it was started with, to start it again after an update.
#[cfg(desktop)]
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
async fn changelog(hub: HubState<'_>, game_id: String) -> Answer<Value> {
    work(&hub, move |hub| hub.changelog(&game_id)).await
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

/// Put the downloaded update in place of this exe, start it, and close this copy. (On Android,
/// hand the new APK to Android's installer, which replaces the app.)
#[tauri::command]
async fn apply_update(app: AppHandle, hub: HubState<'_>) -> Answer<()> {
    #[cfg(mobile)]
    {
        let _ = app;
        log::line("Update handed to Android's installer");
        return work(&hub, |hub| hub.install_update()).await;
    }
    #[cfg(desktop)]
    {
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
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Answer<Option<String>> {
    #[cfg(mobile)]
    {
        let _ = app;
        return Ok(None); // a phone has no install folder to pick
    }
    #[cfg(desktop)]
    let chosen = tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(|error| format!("Could not open the folder picker: {error}"))?;
    #[cfg(desktop)]
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

/// Toki Hub starts here, on a computer (from main.rs) and on a phone.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(desktop)]
    desktop_run();
    #[cfg(mobile)]
    mobile_run();
}

#[cfg(desktop)]
fn desktop_run() {
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
    // After an update, wait for the old copy to close first: it may still have files open in the
    // data folder, and the two must never share the window's browser engine.
    let exe = std::env::current_exe().ok();
    let old_exe_gone = match (&exe, just_updated) {
        (Some(exe), true) => Some(remove_old_exe(exe, std::time::Duration::from_secs(15))),
        _ => None,
    };
    let (data_dir, moved) = match data_dir {
        Some(dir) => (dir, Vec::new()),
        None => migrate::settle_data_dir(&paths::default_data_dir()),
    };
    log::start(&data_dir.join("logs").join("hub.log"));
    for line in moved {
        log::line(&line);
    }
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

    log::line(&format!("Toki Hub {} starting (data: {})", env!("CARGO_PKG_VERSION"), data_dir.display()));
    match old_exe_gone {
        Some(gone) => log::line(if gone { "Finished updating" } else { "The old version is still closing" }),
        None => {
            if let Some(exe) = exe.clone() {
                std::thread::spawn(move || remove_old_exe(&exe, std::time::Duration::from_secs(30)));
            }
        }
    }
    if let Some(exe) = exe {
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
        .invoke_handler(commands())
        .run(tauri::generate_context!());
    if let Err(error) = result {
        log::line(&format!("Toki Hub could not start: {error}"));
        std::process::exit(1);
    }
}

fn commands() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        state, poll, reload, changelog, install, cancel, uninstall, launch, stop,
        open_folder, open_log, open_place, save_settings, clear_token, pick_folder,
        check_for_updates, download_update, apply_update
    ]
}

/// The phone, as Toki Hub's core sees it (see core/src/device.rs).
#[cfg(target_os = "android")]
struct AppDevice(AppHandle);

#[cfg(target_os = "android")]
impl AppDevice {
    fn phone(&self) -> Result<tauri::State<'_, tauri_plugin_tokidevice::Phone<tauri::Wry>>, HubError> {
        tauri_plugin_tokidevice::phone(&self.0).ok_or_else(|| HubError("Toki Hub is still starting.".into()))
    }
}

#[cfg(target_os = "android")]
impl toki_hub_core::device::Device for AppDevice {
    fn installed(&self, package: &str) -> Option<String> {
        self.phone().ok()?.installed(package)
    }
    fn install(&self, apk: &Path) -> Result<(), HubError> {
        self.phone()?.install(&apk.to_string_lossy()).map_err(HubError)
    }
    fn launch(&self, package: &str) -> Result<(), HubError> {
        self.phone()?.launch(package).map_err(HubError)
    }
    fn uninstall(&self, package: &str) -> Result<(), HubError> {
        self.phone()?.uninstall(package).map_err(HubError)
    }
}

#[cfg(mobile)]
fn mobile_run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_tokidevice::init())
        .register_uri_scheme_protocol("tokicatalog", |context, request| catalog_image(context.app_handle(), &request))
        .setup(|app| {
            // The app's own files: where Android's installer can be shown the downloaded APKs
            let files = tauri_plugin_tokidevice::phone(app.handle())
                .and_then(|phone| phone.folders().ok())
                .map(|folders| PathBuf::from(folders.files))
                .or_else(|| app.path().app_data_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            let data_dir = files.join("hub");
            log::start(&data_dir.join("logs").join("hub.log"));
            log::line(&format!("Toki Hub {} starting on Android (data: {})", env!("CARGO_PKG_VERSION"), data_dir.display()));
            let mut options = HubOptions::new(catalog_dirs(&data_dir), data_dir);
            options.hub_version = env!("CARGO_PKG_VERSION").to_string();
            options.updates = Some(UpdateSource {
                repo: UPDATE_REPO.into(),
                tag_prefix: UPDATE_TAG_PREFIX.into(),
                asset_suffix: UPDATE_ASSET_SUFFIX.into(),
            });
            options.opener = Arc::new(|path: &Path| Err(HubError(format!("A phone can't open {}.", path.display()))));
            #[cfg(target_os = "android")]
            {
                options.device = Some(Arc::new(AppDevice(app.handle().clone())));
            }
            app.manage(Arc::new(Hub::new(options)));
            Ok(())
        })
        .invoke_handler(commands())
        .run(tauri::generate_context!());
    if let Err(error) = result {
        log::line(&format!("Toki Hub could not start: {error}"));
    }
}

//! Updating Toki Hub itself from its GitHub releases.
//!
//! 1. `check` asks the GitHub API for the repository's releases and picks the newest one tagged
//!    `<prefix><version>` (for example `tauri-v0.3.0`) that is newer than this copy and has a file
//!    for this computer. The repository is private, so this uses the token from Settings.
//! 2. `start_download` downloads that file and checks it against the sha256 that GitHub records
//!    for every release file, so a damaged or swapped download is never used.
//! 3. The app then calls `swap_exe` and restarts: a running exe cannot be overwritten on Windows,
//!    but it can be renamed, so the old one becomes `Toki Hub.exe.old` and the new one takes its
//!    name. The new copy deletes the `.old` file when it starts.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::download::fetch;
use crate::installer::make_executable;
use crate::settings::Settings;
use crate::sources::{github_error, quote, Download};
use crate::versions::version_key;
use crate::{fail, HubError, Result};

/// Where updates come from.
#[derive(Debug, Clone)]
pub struct UpdateSource {
    /// "Owner/Repo".
    pub repo: String,
    /// Release tags look like `<tag_prefix><version>`.
    pub tag_prefix: String,
    /// The release file for this computer ends with this, like ".exe".
    pub asset_suffix: String,
}

#[derive(Debug, Clone, Default)]
struct State {
    status: &'static str,
    version: String,
    notes: String,
    url: String,
    sha256: String,
    received: u64,
    total: u64,
    error: String,
    file: Option<PathBuf>,
}

pub struct Updater {
    source: Option<UpdateSource>,
    current: String,
    github_api: String,
    settings: Arc<Settings>,
    agent: ureq::Agent,
    downloads_dir: PathBuf,
    state: Mutex<State>,
}

impl Updater {
    pub fn new(
        source: Option<UpdateSource>,
        current: &str,
        github_api: &str,
        settings: Arc<Settings>,
        agent: ureq::Agent,
        downloads_dir: PathBuf,
    ) -> Arc<Updater> {
        let status = if source.is_some() { "idle" } else { "unsupported" };
        Arc::new(Updater {
            source,
            current: current.to_string(),
            github_api: github_api.trim_end_matches('/').to_string(),
            settings,
            agent,
            downloads_dir,
            state: Mutex::new(State { status, ..Default::default() }),
        })
    }

    pub fn to_json(&self) -> Value {
        let state = self.state.lock().unwrap();
        json!({
            "status": state.status, "current": self.current, "version": state.version, "notes": state.notes,
            "received": state.received, "total": state.total, "error": state.error,
        })
    }

    /// The downloaded and verified new exe, once `status` is "ready".
    pub fn ready_file(&self) -> Option<PathBuf> {
        let state = self.state.lock().unwrap();
        if state.status == "ready" { state.file.clone() } else { None }
    }

    /// Look for a newer release. Blocks while it asks GitHub.
    pub fn check(&self) -> Value {
        let Some(source) = &self.source else { return self.to_json() };
        if matches!(self.state.lock().unwrap().status, "downloading" | "ready") {
            return self.to_json();
        }
        self.state.lock().unwrap().status = "checking";
        let found = self.newest_release(source);
        {
            let mut state = self.state.lock().unwrap();
            match found {
                Ok(Some(release)) => {
                    *state = State { status: "available", ..release };
                }
                Ok(None) => *state = State { status: "current", ..Default::default() },
                Err(error) => *state = State { status: "error", error: error.0, ..Default::default() },
            }
        }
        self.to_json()
    }

    fn newest_release(&self, source: &UpdateSource) -> Result<Option<State>> {
        let url = format!("{}/repos/{}/releases?per_page=50", self.github_api, source.repo);
        let mut request = self.agent.get(&url).header("Accept", "application/vnd.github+json");
        let token = self.settings.github_token();
        if !token.is_empty() {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let mut response = request.call().map_err(|e| HubError(format!("Could not reach GitHub: {e}")))?;
        if !response.status().is_success() {
            return Err(github_error(&response, &format!("releases of {}", source.repo)));
        }
        let releases: Value = response
            .body_mut()
            .read_to_string()
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or_else(|| HubError("GitHub sent an answer Toki Hub could not read.".into()))?;
        let mut best: Option<State> = None;
        for release in releases.as_array().into_iter().flatten() {
            let tag = release["tag_name"].as_str().unwrap_or_default();
            let Some(version) = tag.strip_prefix(&source.tag_prefix) else { continue };
            let key = version_key(version);
            if release["draft"].as_bool() == Some(true) || key.0.is_empty() || key <= version_key(&self.current) {
                continue;
            }
            if best.as_ref().is_some_and(|b| version_key(&b.version) >= key) {
                continue;
            }
            let asset = release["assets"].as_array().into_iter().flatten().find(|asset| {
                asset["name"].as_str().is_some_and(|name| name.to_lowercase().ends_with(&source.asset_suffix.to_lowercase()))
            });
            let Some(asset) = asset else { continue };
            let Some(sha256) = asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")) else { continue };
            best = Some(State {
                version: version.to_string(),
                notes: release["body"].as_str().unwrap_or_default().to_string(),
                url: asset["url"].as_str().unwrap_or_default().to_string(),
                sha256: sha256.to_string(),
                total: asset["size"].as_u64().unwrap_or(0),
                ..Default::default()
            });
        }
        Ok(best)
    }

    /// Download the update found by `check`, in the background.
    pub fn start_download(self: &Arc<Self>) -> Result<Value> {
        let (version, plan) = {
            let mut state = self.state.lock().unwrap();
            match state.status {
                "available" => {}
                "downloading" | "ready" => return Ok(self.json_of(&state)),
                _ => return fail("There is no update to download. Check for updates first."),
            }
            state.status = "downloading";
            state.received = 0;
            let mut headers = vec![("Accept", "application/octet-stream".to_string())];
            let token = self.settings.github_token();
            if !token.is_empty() {
                headers.push(("Authorization", format!("Bearer {token}")));
            }
            let plan = Download {
                url: state.url.clone(),
                filename: format!("Toki Hub {}", state.version),
                headers,
                github_what: format!("update to version {}", state.version),
                sha256: state.sha256.clone(),
            };
            (state.version.clone(), plan)
        };
        // (Android's installer wants an APK to be called one)
        let apk = self.source.as_ref().is_some_and(|source| source.asset_suffix.ends_with(".apk"));
        let updater = Arc::clone(self);
        std::thread::Builder::new()
            .name("update".into())
            .spawn(move || {
                let file = updater.downloads_dir.join(format!("update-{}{}", quote(&version), if apk { ".apk" } else { "" }));
                let progress = |received: u64, total: u64| {
                    let mut state = updater.state.lock().unwrap();
                    state.received = received;
                    if total > 0 {
                        state.total = total;
                    }
                };
                let result = fetch(&updater.agent, &plan, &file, &progress, &|| Ok(()));
                let mut state = updater.state.lock().unwrap();
                match result {
                    Ok(()) => {
                        state.status = "ready";
                        state.file = Some(file);
                    }
                    Err(error) => {
                        state.status = "error";
                        state.error = error.0;
                    }
                }
            })
            .map_err(|e| HubError(format!("Could not start the download: {e}")))?;
        Ok(self.to_json())
    }

    fn json_of(&self, state: &State) -> Value {
        json!({"status": state.status, "current": self.current, "version": state.version})
    }
}

/// Where the replaced exe goes: `Toki Hub.exe` -> `Toki Hub.exe.old`.
pub fn old_exe(current: &Path) -> PathBuf {
    let mut name = current.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    current.with_file_name(name)
}

/// Put `new_file` where the running exe `current` is. The running exe is renamed out of the way
/// (Windows allows renaming, not overwriting, a running exe); if anything fails it is put back.
pub fn swap_exe(current: &Path, new_file: &Path) -> Result<()> {
    let old = old_exe(current);
    let _ = std::fs::remove_file(&old);
    let folder = current.parent().map(|f| f.display().to_string()).unwrap_or_default();
    std::fs::rename(current, &old).map_err(|e| {
        HubError(format!(
            "Toki Hub cannot update itself in {folder} ({e}). Move Toki Hub.exe to a folder of your own, \
             like Documents, or download the new version from GitHub."
        ))
    })?;
    if let Err(error) = std::fs::copy(new_file, current) {
        let _ = std::fs::remove_file(current);
        let _ = std::fs::rename(&old, current);
        return fail(format!("Could not put the new version in place: {error}"));
    }
    make_executable(current);
    let _ = std::fs::remove_file(new_file);
    Ok(())
}

/// Delete the `.old` exe left by an update, waiting up to `wait` for the old copy to finish
/// closing (until then Windows will not let it be deleted). True once it is gone.
pub fn remove_old_exe(current: &Path, wait: Duration) -> bool {
    let old = old_exe(current);
    let deadline = std::time::Instant::now() + wait;
    loop {
        if !old.exists() || std::fs::remove_file(&old).is_ok() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

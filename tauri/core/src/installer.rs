//! Downloading and installing game versions.
//!
//! Each version installs into its own folder, `<install dir>/<game id>/<version>/`, so any number
//! of versions sit side by side. Installs run in a background thread and are atomic: files are
//! unpacked into a staging folder first and only moved into place once everything succeeded, so a
//! failed or cancelled download never leaves a half-installed version behind.
//!
//! An installed version is recognised by the `.tokihub.json` marker in its folder, which also
//! records how to launch it. The layout matches the Python version's.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::catalog::{Build, Game, Version};
use crate::paths::{is_within, safe_name};
use crate::settings::Settings;
use crate::sources::{github_error, Download, LocalFiles, Plan, SourceResolver};
use crate::{fail, HubError, Result};

pub const MARKER: &str = ".tokihub.json";
const CHUNK: usize = 256 * 1024;
/// Leftovers of an interrupted install older than this are swept away at start-up.
const STALE_AFTER: Duration = Duration::from_secs(3600);

pub type IsRunning = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

pub struct Task {
    pub id: String,
    pub game_id: String,
    pub title: String,
    pub version: String,
    status: Mutex<&'static str>,
    error: Mutex<String>,
    received: AtomicU64,
    total: AtomicU64,
    cancel_requested: AtomicBool,
}

impl Task {
    pub fn status(&self) -> &'static str {
        *self.status.lock().unwrap()
    }

    fn set_status(&self, status: &'static str) {
        *self.status.lock().unwrap() = status;
    }

    pub fn active(&self) -> bool {
        matches!(self.status(), "queued" | "downloading" | "installing")
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "game_id": self.game_id, "title": self.title, "version": self.version,
            "status": self.status(), "error": *self.error.lock().unwrap(),
            "received": self.received.load(Ordering::Relaxed), "total": self.total.load(Ordering::Relaxed),
        })
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancel_requested.load(Ordering::Relaxed) { Err(HubError(CANCELLED.into())) } else { Ok(()) }
    }
}

const CANCELLED: &str = "\u{0}cancelled";

pub struct Installer {
    settings: Arc<Settings>,
    resolver: Arc<SourceResolver>,
    downloads_dir: PathBuf,
    is_running: IsRunning,
    platform: String,
    tasks: Mutex<HashMap<(String, String), Arc<Task>>>,
    next_id: AtomicU64,
}

impl Installer {
    pub fn new(
        settings: Arc<Settings>,
        resolver: Arc<SourceResolver>,
        downloads_dir: PathBuf,
        is_running: IsRunning,
        platform: &str,
    ) -> Installer {
        let installer = Installer {
            settings,
            resolver,
            downloads_dir,
            is_running,
            platform: platform.to_string(),
            tasks: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        };
        installer.sweep_leftovers();
        installer
    }

    pub fn version_dir(&self, game_id: &str, version: &str) -> PathBuf {
        self.settings.install_dir().join(game_id).join(safe_name(version))
    }

    /// {game_id: {version: marker info}} for everything in the install folder.
    pub fn installed(&self) -> BTreeMap<String, BTreeMap<String, Value>> {
        let mut found: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
        let Ok(games) = std::fs::read_dir(self.settings.install_dir()) else { return found };
        for game in games.flatten() {
            let Ok(versions) = std::fs::read_dir(game.path()) else { continue };
            for version in versions.flatten() {
                if version.file_name().to_string_lossy().starts_with('.') {
                    continue; // an install still in progress
                }
                if let Some(info) = read_marker(&version.path()) {
                    let game_id = info["game"].as_str().unwrap_or_default().to_string();
                    let name = info["version"].as_str().unwrap_or_default().to_string();
                    found.entry(game_id).or_default().insert(name, info);
                }
            }
        }
        found
    }

    pub fn installed_version(&self, game_id: &str, version: &str) -> Option<Value> {
        read_marker(&self.version_dir(game_id, version))
    }

    pub fn tasks(&self) -> Vec<Value> {
        let tasks = self.tasks.lock().unwrap();
        let mut list: Vec<&Arc<Task>> = tasks.values().collect();
        list.sort_by_key(|task| task.id[1..].parse::<u64>().unwrap_or(0));
        list.iter().map(|task| task.to_json()).collect()
    }

    pub fn start(&self, game: &Game, version: &Version) -> Result<Value> {
        let Some(build) = version.build_for(&self.platform).cloned() else {
            return fail(format!("{} {} has no build for {}.", game.title, version.version, self.platform));
        };
        if (self.is_running)(&game.id, &version.version) {
            return fail(format!("Close {} {} before reinstalling it.", game.title, version.version));
        }
        let key = (game.id.clone(), version.version.clone());
        let task = {
            let mut tasks = self.tasks.lock().unwrap();
            if tasks.get(&key).is_some_and(|task| task.active()) {
                return fail(format!("{} {} is already downloading.", game.title, version.version));
            }
            let task = Arc::new(Task {
                id: format!("t{}", self.next_id.fetch_add(1, Ordering::Relaxed)),
                game_id: game.id.clone(),
                title: game.title.clone(),
                version: version.version.clone(),
                status: Mutex::new("queued"),
                error: Mutex::new(String::new()),
                received: AtomicU64::new(0),
                total: AtomicU64::new(0),
                cancel_requested: AtomicBool::new(false),
            });
            tasks.insert(key, task.clone());
            task
        };
        let job = Job {
            task: task.clone(),
            game: game.clone(),
            version: version.clone(),
            build,
            target: self.version_dir(&game.id, &version.version),
            downloads_dir: self.downloads_dir.clone(),
            resolver: self.resolver.clone(),
            is_running: self.is_running.clone(),
        };
        std::thread::Builder::new()
            .name(format!("install-{}", task.id))
            .spawn(move || job.run())
            .map_err(|e| HubError(format!("Could not start the download: {e}")))?;
        Ok(task.to_json())
    }

    pub fn cancel(&self, task_id: &str) -> bool {
        let tasks = self.tasks.lock().unwrap();
        match tasks.values().find(|task| task.id == task_id && task.active()) {
            Some(task) => {
                task.cancel_requested.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn uninstall(&self, game_id: &str, version: &str) -> Result<()> {
        let folder = self.version_dir(game_id, version);
        if !folder.join(MARKER).is_file() {
            return fail(format!("Version {version} is not installed."));
        }
        if (self.is_running)(game_id, version) {
            return fail("Close the game before uninstalling it.");
        }
        remove_tree(&folder).map_err(|e| HubError(format!("Could not delete {}: {e}", folder.display())))?;
        if let Some(parent) = folder.parent() {
            let _ = std::fs::remove_dir(parent); // tidy up the game's folder once its last version is gone
        }
        Ok(())
    }

    fn sweep_leftovers(&self) {
        let cutoff = SystemTime::now() - STALE_AFTER;
        let stale = |path: &Path| {
            path.metadata().and_then(|m| m.modified()).map(|time| time < cutoff).unwrap_or(false)
        };
        if let Ok(games) = std::fs::read_dir(self.settings.install_dir()) {
            for game in games.flatten() {
                for entry in std::fs::read_dir(game.path()).into_iter().flatten().flatten() {
                    let path = entry.path();
                    if entry.file_name().to_string_lossy().starts_with(".staging-") && stale(&path) {
                        let _ = remove_tree(&path);
                    }
                }
            }
        }
        for entry in std::fs::read_dir(&self.downloads_dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "part") && stale(&path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// One install, run in its own thread.
struct Job {
    task: Arc<Task>,
    game: Game,
    version: Version,
    build: Build,
    target: PathBuf,
    downloads_dir: PathBuf,
    resolver: Arc<SourceResolver>,
    is_running: IsRunning,
}

impl Job {
    fn run(self) {
        let staging = self
            .target
            .with_file_name(format!(".staging-{}-{}", safe_name(&self.version.version), self.task.id));
        let mut temp_file = None;
        let result = self.install(&staging, &mut temp_file);
        if staging.exists() {
            let _ = remove_tree(&staging);
        }
        if let Some(file) = temp_file {
            let _ = std::fs::remove_file(file);
        }
        match result {
            Ok(()) => self.task.set_status("done"),
            Err(error) if error.0 == CANCELLED => self.task.set_status("cancelled"),
            Err(error) => {
                *self.task.error.lock().unwrap() = error.0;
                self.task.set_status("error");
            }
        }
    }

    fn install(&self, staging: &Path, temp_file: &mut Option<PathBuf>) -> Result<()> {
        let task = &self.task;
        match self.resolver.resolve(&self.build.source, &self.game.folder)? {
            Plan::Download(plan) => {
                task.set_status("downloading");
                let file = self.download(&plan)?;
                *temp_file = Some(file.clone());
                task.set_status("installing");
                std::fs::create_dir_all(staging)?;
                if plan.filename.to_lowercase().ends_with(".zip") {
                    extract_zip(&file, staging, &|| task.check_cancel())?;
                } else {
                    let dest = staging.join(&plan.filename);
                    std::fs::copy(&file, &dest)?;
                    make_executable(&dest);
                }
            }
            Plan::Local(local) => {
                task.set_status("installing");
                copy_local(&local, staging, &|| task.check_cancel())?;
            }
        }
        task.check_cancel()?;
        let launch_file = staging.join(&self.build.launch.path);
        if !is_within(&launch_file, staging) || !launch_file.exists() {
            return fail(format!(
                "The downloaded files do not contain '{}', which the catalog says to launch. Check `launch.path` for this build.",
                self.build.launch.path
            ));
        }
        write_marker(staging, &self.game, &self.version, &self.build)?;
        if (self.is_running)(&self.game.id, &self.version.version) {
            return fail(format!(
                "Close {} {} so the new files can be put in place.",
                self.game.title, self.version.version
            ));
        }
        if self.target.exists() {
            remove_tree(&self.target)?;
        }
        move_into_place(staging, &self.target)?;
        Ok(())
    }

    fn download(&self, plan: &Download) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.downloads_dir)?;
        let part = self.downloads_dir.join(format!("{}-{}.part", self.task.id, plan.filename));
        let result = self.receive(plan, &part);
        if result.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        result.map(|_| part)
    }

    fn receive(&self, plan: &Download, part: &Path) -> Result<()> {
        let task = &self.task;
        let mut request = self.resolver.agent().get(&plan.url);
        for (key, value) in &plan.headers {
            request = request.header(*key, value);
        }
        let response = request.call().map_err(|e| {
            HubError(format!("Could not connect to download {}: {e}", plan.filename))
        })?;
        if !response.status().is_success() {
            if !plan.github_what.is_empty() {
                return Err(github_error(&response, &plan.github_what));
            }
            let status = response.status();
            return fail(format!(
                "Download failed: HTTP {} {} ({})",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
                plan.url
            ));
        }
        let total = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        task.total.store(total, Ordering::Relaxed);
        let mut reader = response.into_body().into_reader();
        let mut out = File::create(part)?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0u8; CHUNK];
        let mut received = 0u64;
        loop {
            task.check_cancel()?;
            let count = reader
                .read(&mut buffer)
                .map_err(|e| HubError(format!("The download was interrupted: {e}")))?;
            if count == 0 {
                break;
            }
            out.write_all(&buffer[..count])?;
            digest.update(&buffer[..count]);
            received += count as u64;
            task.received.store(received, Ordering::Relaxed);
        }
        out.flush()?;
        if total > 0 && received < total {
            return fail("The download ended early. Please try again.");
        }
        if !plan.sha256.is_empty() && hex(&digest.finalize()) != plan.sha256 {
            return fail(format!(
                "{} does not match the sha256 in the catalog, so it was not installed. The file may be damaged, or the catalog entry needs updating.",
                plan.filename
            ));
        }
        Ok(())
    }
}

fn copy_local(local: &LocalFiles, staging: &Path, check_cancel: &dyn Fn() -> Result<()>) -> Result<()> {
    let source = &local.path;
    if !local.sha256.is_empty() && source.is_file() && file_sha256(source)? != local.sha256 {
        return fail(format!(
            "{} does not match the sha256 in the catalog, so it was not installed.",
            source.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    if source.is_dir() {
        copy_tree(source, staging, check_cancel)?;
    } else if source.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        std::fs::create_dir_all(staging)?;
        extract_zip(source, staging, check_cancel)?;
    } else {
        std::fs::create_dir_all(staging)?;
        std::fs::copy(source, staging.join(source.file_name().unwrap_or_default()))?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path, check_cancel: &dyn Fn() -> Result<()>) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        check_cancel()?;
        let entry = entry?;
        let name = entry.file_name();
        if [".git", "__pycache__", MARKER].iter().any(|skip| name == *skip) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_tree(&path, &to.join(&name), check_cancel)?;
        } else {
            std::fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

/// Extract a zip into `dest`. A single top-level folder (as in GitHub source zips) is dropped, so
/// the game's files land directly in `dest`. Entries that would escape `dest` are refused.
pub fn extract_zip(zip_path: &Path, dest: &Path, check_cancel: &dyn Fn() -> Result<()>) -> Result<()> {
    let damaged = |e: zip::result::ZipError| HubError(format!("The zip archive is damaged ({e})."));
    let mut archive = zip::ZipArchive::new(File::open(zip_path)?)
        .map_err(|_| HubError("The downloaded file is not a valid zip archive.".into()))?;
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(damaged)?;
        if !file.is_dir() {
            let name = file.name().map_err(damaged)?.replace('\\', "/");
            entries.push((i, name, file.unix_mode()));
        }
    }
    if entries.is_empty() {
        return fail("The zip archive is empty.");
    }
    let first_top = entries[0].1.split('/').next().unwrap_or_default().to_string();
    let strip = entries.iter().all(|(_, name, _)| name.contains('/') && name.split('/').next() == Some(&first_top));
    let cut = if strip { first_top.len() + 1 } else { 0 };
    for (index, name, mode) in entries {
        check_cancel()?;
        let relative = &name[cut..];
        let target = dest.join(relative);
        if relative.is_empty()
            || relative.starts_with('/')
            || relative.contains(':')
            || relative.split('/').any(|part| part == "..")
            || !is_within(&target, dest)
        {
            return fail(format!("The zip archive contains an unsafe path ({name}), so it was not installed."));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = archive.by_index(index).map_err(damaged)?;
        let mut out = File::create(&target)?;
        std::io::copy(&mut file, &mut out).map_err(|e| HubError(format!("The zip archive is damaged ({name}: {e}).")))?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            if mode & 0o111 != 0 {
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode((mode & 0o777) | 0o600))?;
            }
        }
        #[cfg(not(unix))]
        let _ = mode;
    }
    Ok(())
}

fn read_marker(folder: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(folder.join(MARKER)).ok()?;
    let mut info: Value = serde_json::from_str(&text).ok()?;
    if !info["game"].is_string() || !info["version"].is_string() {
        return None;
    }
    info["path"] = Value::String(folder.to_string_lossy().into_owned());
    Some(info)
}

fn write_marker(folder: &Path, game: &Game, version: &Version, build: &Build) -> Result<()> {
    let marker = json!({
        "game": game.id,
        "version": version.version,
        "platform": build.platform,
        "source": build.source,
        "launch": build.launch,
        "installed_at": utc_now(),
        "size": folder_size(folder),
    });
    std::fs::write(folder.join(MARKER), serde_json::to_string_pretty(&marker).unwrap())?;
    Ok(())
}

fn folder_size(folder: &Path) -> u64 {
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => folder_size(&entry.path()),
            _ => entry.metadata().map(|m| m.len()).unwrap_or(0),
        })
        .sum()
}

/// Rename the finished staging folder to its final name. On Windows, antivirus scanners often hold
/// freshly written files open for a moment, so a refused rename is retried briefly.
fn move_into_place(staging: &Path, target: &Path) -> Result<()> {
    for attempt in 0..6 {
        match std::fs::rename(staging, target) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied && attempt < 5 => {
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Delete a folder, including read-only files (common on Windows), retrying briefly in case a
/// just-closed game still holds a file open.
pub fn remove_tree(path: &Path) -> std::io::Result<()> {
    let mut last_error = None;
    for _ in 0..3 {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                make_writable(path);
                last_error = Some(error);
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
    Err(last_error.unwrap())
}

fn make_writable(path: &Path) {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        let mut permissions = metadata.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = std::fs::set_permissions(path, permissions);
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
                make_writable(&entry.path());
            }
        }
    }
}

pub fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mode = metadata.permissions().mode() | 0o111;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

pub fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex(&digest.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Now, as "2026-11-09T15:05:18+00:00".
pub fn utc_now() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rest) = ((seconds / 86_400) as i64, seconds % 86_400);
    // Howard Hinnant's days-to-civil algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}+00:00",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

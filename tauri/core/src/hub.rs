//! Everything the UI can ask for. Every method returns plain JSON, shaped like the Python
//! version's API so the same UI works on top of it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde_json::{json, Map, Value};

use crate::catalog::{Build, Catalog, Game, IMAGE_TYPES};
use crate::installer::Installer;
use crate::launcher::{system_opener, Launcher, Opener};
use crate::paths::{expand_home, is_within, which};
use crate::settings::Settings;
use crate::sources::{SourceResolver, GITHUB_API};
use crate::{fail, Result};

pub struct HubOptions {
    /// Shown in Settings.
    pub hub_version: String,
    /// Catalog folders in order; later ones override earlier ones, and the last is the one people
    /// add games to.
    pub catalog_dirs: Vec<PathBuf>,
    pub data_dir: PathBuf,
    pub github_api: String,
    pub platform: String,
    pub opener: Opener,
}

impl HubOptions {
    pub fn new(catalog_dirs: Vec<PathBuf>, data_dir: PathBuf) -> HubOptions {
        HubOptions {
            hub_version: env!("CARGO_PKG_VERSION").to_string(),
            catalog_dirs,
            data_dir,
            github_api: GITHUB_API.to_string(),
            platform: crate::paths::current_platform().to_string(),
            opener: system_opener(),
        }
    }
}

pub struct Hub {
    hub_version: String,
    catalog_dirs: Vec<PathBuf>,
    data_dir: PathBuf,
    platform: String,
    opener: Opener,
    pub settings: Arc<Settings>,
    pub launcher: Arc<Launcher>,
    pub installer: Installer,
    catalog: RwLock<Arc<Catalog>>,
}

impl Hub {
    pub fn new(options: HubOptions) -> Hub {
        let data_dir = options.data_dir;
        let settings = Arc::new(Settings::load(&data_dir.join("settings.json"), &data_dir));
        let token_settings = settings.clone();
        let resolver = Arc::new(SourceResolver::new(Box::new(move || token_settings.github_token()), &options.github_api));
        let launcher = Launcher::new(settings.clone(), data_dir.join("logs"), options.opener.clone());
        let watcher = launcher.clone();
        let installer = Installer::new(
            settings.clone(),
            resolver,
            data_dir.join("downloads"),
            Arc::new(move |game_id: &str, version: &str| watcher.is_running(game_id, version)),
            &options.platform,
        );
        let catalog = Catalog::load(&options.catalog_dirs);
        Hub {
            hub_version: options.hub_version,
            catalog_dirs: options.catalog_dirs,
            data_dir,
            platform: options.platform,
            opener: options.opener,
            settings,
            launcher,
            installer,
            catalog: RwLock::new(Arc::new(catalog)),
        }
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        self.catalog.read().unwrap().clone()
    }

    /// The catalog folder people add games to.
    pub fn catalog_dir(&self) -> PathBuf {
        self.catalog_dirs.last().cloned().unwrap_or_default()
    }

    // -- Reading state --

    /// Everything the UI needs to draw itself from scratch.
    pub fn state(&self) -> Value {
        let catalog = self.catalog();
        let mut state = json!({
            "hub_version": self.hub_version,
            "platform": self.platform,
            "games": catalog.games.iter().map(|game| self.game_json(game)).collect::<Vec<_>>(),
            "problems": catalog.problems,
            "settings": self.settings.for_ui(),
            "places": {"catalog": self.catalog_dir(), "data": self.data_dir},
            "native_window": true,
            "frozen": true,
        });
        if let (Value::Object(state), Value::Object(changing)) = (&mut state, self.poll(0)) {
            state.extend(changing);
        }
        state
    }

    /// The parts of the state that change while the hub is open.
    pub fn poll(&self, after_event: u64) -> Value {
        let installed: Map<String, Value> = self
            .installer
            .installed()
            .into_iter()
            .map(|(game_id, versions)| {
                let versions: Map<String, Value> = versions
                    .into_iter()
                    .map(|(version, info)| {
                        let summary = json!({"path": info["path"], "installed_at": info["installed_at"], "size": info["size"]});
                        (version, summary)
                    })
                    .collect();
                (game_id, Value::Object(versions))
            })
            .collect();
        json!({
            "installed": installed,
            "tasks": self.installer.tasks(),
            "running": self.launcher.running(),
            "events": self.launcher.events_since(after_event),
        })
    }

    pub fn reload(&self) -> Value {
        *self.catalog.write().unwrap() = Arc::new(Catalog::load(&self.catalog_dirs));
        self.state()
    }

    // -- Games --

    pub fn install(&self, game_id: &str, version: &str) -> Result<Value> {
        let catalog = self.catalog();
        let game = catalog.game(game_id)?;
        self.installer.start(game, game.version(version)?)
    }

    pub fn cancel(&self, task_id: &str) -> bool {
        self.installer.cancel(task_id)
    }

    pub fn uninstall(&self, game_id: &str, version: &str) -> Result<()> {
        self.installer.uninstall(game_id, version)
    }

    pub fn launch(&self, game_id: &str, version: &str) -> Result<Value> {
        let catalog = self.catalog();
        let game = catalog.game(game_id)?;
        let Some(installed) = self.installer.installed_version(game_id, version) else {
            return fail(format!("{} {version} is not installed yet.", game.title));
        };
        self.launcher.launch(game_id, &game.title, version, &installed)
    }

    pub fn stop(&self, game_id: &str, version: &str) -> Result<()> {
        self.launcher.stop(game_id, version)
    }

    pub fn open_folder(&self, game_id: &str, version: &str) -> Result<()> {
        match self.installer.installed_version(game_id, version) {
            Some(installed) => (self.opener)(Path::new(installed["path"].as_str().unwrap_or_default())),
            None => fail("That version is not installed."),
        }
    }

    pub fn open_log(&self, game_id: &str, version: &str) -> Result<()> {
        let log = self.launcher.log_path(game_id, version);
        if !log.is_file() {
            return fail("There is no log yet: launch the game first.");
        }
        (self.opener)(&log)
    }

    pub fn open_place(&self, which: &str) -> Result<()> {
        let place = match which {
            "catalog" => self.catalog_dir(),
            "data" => self.data_dir.clone(),
            "games" => self.settings.install_dir(),
            "logs" => self.launcher.logs_dir().to_path_buf(),
            _ => return fail(format!("Unknown place '{which}'.")),
        };
        std::fs::create_dir_all(&place)?;
        (self.opener)(&place)
    }

    // -- Settings --

    pub fn save_settings(&self, changes: &Value) -> Result<Value> {
        let Value::Object(changes) = changes else { return fail("Settings must be sent as an object.") };
        let text = |key: &str| changes.get(key).and_then(Value::as_str).map(|v| v.trim().to_string());
        let mut clean: Vec<(&str, String)> = Vec::new();
        if let Some(folder) = text("install_dir") {
            if !folder.is_empty() && !expand_home(&folder).is_absolute() {
                return fail("The install folder must be a full path, like C:\\Games\\Toki Hub.");
            }
            clean.push(("install_dir", folder));
        }
        if let Some(python) = text("python") {
            if !python.is_empty() && !Path::new(&python).is_file() && which(&python).is_none() {
                return fail(format!("Python interpreter not found: {python}"));
            }
            clean.push(("python", python));
        }
        if let Some(token) = text("github_token").filter(|t| !t.is_empty()) {
            clean.push(("github_token", token));
        }
        self.settings.update(&clean)?;
        Ok(self.settings.for_ui())
    }

    pub fn clear_token(&self) -> Result<Value> {
        self.settings.update(&[("github_token", String::new())])?;
        Ok(self.settings.for_ui())
    }

    /// A catalog image, for the UI: (bytes, content type). Only images inside a game's folder.
    pub fn image(&self, game_id: &str, relative: &str) -> Option<(Vec<u8>, &'static str)> {
        let catalog = self.catalog();
        let game = catalog.get(game_id)?;
        let file = game.folder.join(relative);
        let extension = file.extension()?.to_string_lossy().to_lowercase();
        if !IMAGE_TYPES.contains(&extension.as_str()) || !is_within(&file, &game.folder) {
            return None;
        }
        let content_type = match extension.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => "image/svg+xml",
        };
        Some((std::fs::read(file).ok()?, content_type))
    }

    // -- Helpers --

    fn game_json(&self, game: &Game) -> Value {
        let image = |path: &Option<PathBuf>| {
            path.as_ref().and_then(|p| p.strip_prefix(&game.folder).ok()).map(|relative| {
                let relative = relative.to_string_lossy().replace('\\', "/");
                format!("{}/{relative}", game.id)
            })
        };
        json!({
            "id": game.id,
            "title": game.title,
            "tagline": game.tagline,
            "description": game.description,
            "developer": game.developer,
            "tags": game.tags,
            "accent": game.accent,
            "icon": image(&game.icon),
            "cover": image(&game.cover),
            "versions": game.versions.iter().map(|v| {
                let mut platforms: Vec<&str> = v.builds.iter().map(|b| b.platform.as_str()).collect();
                platforms.sort();
                platforms.dedup();
                json!({
                    "version": v.version,
                    "date": v.date,
                    "notes": v.notes,
                    "platforms": platforms,
                    "build": v.build_for(&self.platform).map(build_json),
                })
            }).collect::<Vec<_>>(),
        })
    }
}

fn build_json(build: &Build) -> Value {
    json!({
        "platform": build.platform,
        "source": build.source_label(),
        "launch": build.launch.kind,
        "note": build.note,
    })
}

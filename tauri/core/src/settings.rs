//! Hub settings, stored as settings.json in the data folder.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Map, Value};

use crate::paths::{expand_home, which};
use crate::Result;

const KEYS: [&str; 3] = ["install_dir", "python", "github_token"];

pub struct Settings {
    path: PathBuf,
    data_dir: PathBuf,
    values: Mutex<Map<String, Value>>,
}

impl Settings {
    pub fn load(path: &Path, data_dir: &Path) -> Settings {
        let saved: Map<String, Value> = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let mut values = Map::new();
        for key in KEYS {
            let value = saved.get(key).and_then(Value::as_str).unwrap_or_default();
            values.insert(key.to_string(), Value::String(value.to_string()));
        }
        Settings { path: path.to_path_buf(), data_dir: data_dir.to_path_buf(), values: Mutex::new(values) }
    }

    pub fn get(&self, key: &str) -> String {
        let values = self.values.lock().unwrap();
        values.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    /// Apply {key: text} changes (unknown keys are ignored) and save.
    pub fn update(&self, changes: &[(&str, String)]) -> Result<()> {
        let mut values = self.values.lock().unwrap();
        for (key, value) in changes {
            if KEYS.contains(key) {
                values.insert(key.to_string(), Value::String(value.trim().to_string()));
            }
        }
        if let Some(folder) = self.path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&*values).unwrap())?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn install_dir(&self) -> PathBuf {
        let custom = self.get("install_dir");
        if custom.is_empty() { self.data_dir.join("games") } else { expand_home(&custom) }
    }

    pub fn github_token(&self) -> String {
        let saved = self.get("github_token");
        if !saved.is_empty() {
            return saved;
        }
        std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")).unwrap_or_default()
    }

    /// The interpreter for `"launch": {"type": "python"}` games.
    pub fn python_command(&self) -> Vec<String> {
        let custom = self.get("python");
        if custom.is_empty() { default_python() } else { vec![custom] }
    }

    /// Settings as the UI sees them. The token itself never leaves the hub.
    pub fn for_ui(&self) -> Value {
        let saved_token = !self.get("github_token").is_empty();
        json!({
            "install_dir": self.get("install_dir"),
            "install_dir_effective": self.install_dir().to_string_lossy(),
            "python": self.get("python"),
            "python_effective": self.python_command().join(" "),
            "has_token": saved_token,
            "token_from_env": !saved_token && !self.github_token().is_empty(),
        })
    }
}

/// The Python on this computer: the `py` launcher on Windows, otherwise python3 or python.
pub fn default_python() -> Vec<String> {
    if cfg!(windows) && which("py").is_some() {
        return vec!["py".into(), "-3".into()];
    }
    for name in ["python3", "python"] {
        if let Some(found) = which(name) {
            return vec![found.to_string_lossy().into_owned()];
        }
    }
    vec!["python".into()]
}

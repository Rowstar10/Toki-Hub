//! Moving the data folder from its old name, "Toki Hub (Tauri)", to "Toki Hub".
//!
//! Before version 0.2.2 settings and installed games were kept in `%LOCALAPPDATA%\Toki Hub (Tauri)`,
//! because the old Python version of the hub used "Toki Hub". At start-up that folder is renamed.
//! If the Python version's folder is still there, it is moved inside, into "old Python version",
//! so nothing is lost and it can be deleted at leisure.
//!
//! uv stores full paths to Python (in each game's environment, and in its "3.12 -> 3.12.15"
//! shortcuts), so those are updated after the move; otherwise installed Python games would break.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::paths::{expand_home, APP_NAME, OLD_APP_NAME};
use crate::runtime::ENV_DIR;

/// Where the old Python version's data ends up.
pub const PYTHON_LEFTOVERS: &str = "old Python version";

/// The data folder to use (normally `wanted`), after moving an old-named one there, and what
/// happened, for the log. If the folder cannot be moved right now (a game started from it may still
/// be running), the old one is used and the move is tried again next time.
pub fn settle_data_dir(wanted: &Path) -> (PathBuf, Vec<String>) {
    let old = wanted.with_file_name(OLD_APP_NAME);
    let mut log = Vec::new();
    if !old.is_dir() {
        return (wanted.to_path_buf(), log);
    }
    if wanted.exists() && is_hub_data(wanted) {
        log.push(format!("{} is already in use; leaving {} as it is", wanted.display(), old.display()));
        return (wanted.to_path_buf(), log);
    }
    let parked = wanted.with_file_name(format!("{APP_NAME} moving"));
    let leftovers = wanted.exists();
    if leftovers {
        if let Err(error) = std::fs::rename(wanted, &parked) {
            log.push(format!("Could not move the old Python version's folder aside ({error}); using {} for now", old.display()));
            return (old, log);
        }
    }
    if let Err(error) = std::fs::rename(&old, wanted) {
        if leftovers {
            let _ = std::fs::rename(&parked, wanted);
        }
        log.push(format!("Could not rename {} ({error}); using it for now", old.display()));
        return (old, log);
    }
    log.push(format!("Renamed {} to {}", old.display(), wanted.display()));
    let environments = environments(wanted);
    log.extend(repair_environments(&old, wanted, &environments));
    if !environments.is_empty() {
        log.push(format!("Updated {} Python environment(s) for the new folder", environments.len()));
    }
    if leftovers {
        match std::fs::rename(&parked, wanted.join(PYTHON_LEFTOVERS)) {
            Ok(()) => log.push(format!("Moved the old Python version's data into \"{PYTHON_LEFTOVERS}\"")),
            Err(error) => log.push(format!("Could not move {} ({error})", parked.display())),
        }
    }
    (wanted.to_path_buf(), log)
}

/// This hub always makes a "cache" folder; the old Python version never did.
fn is_hub_data(dir: &Path) -> bool {
    ["cache", "runtimes", "tools"].iter().any(|name| dir.join(name).is_dir())
}

/// Every game environment: in the default install folder and in one chosen in Settings.
fn environments(data_dir: &Path) -> Vec<PathBuf> {
    let mut install_dirs = vec![data_dir.join("games")];
    let custom = std::fs::read_to_string(data_dir.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|settings| settings["install_dir"].as_str().map(str::to_string))
        .filter(|dir| !dir.is_empty());
    if let Some(custom) = custom {
        install_dirs.push(expand_home(&custom));
    }
    let mut found = Vec::new();
    for dir in install_dirs {
        for game in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            for version in std::fs::read_dir(game.path()).into_iter().flatten().flatten() {
                let env = version.path().join(ENV_DIR);
                if env.join("pyvenv.cfg").is_file() {
                    found.push(env);
                }
            }
        }
    }
    found
}

/// After the data folder moved from `old` to `new`, point uv's Python shortcuts and the games'
/// environments at the new place. Returns problems, for the log.
pub fn repair_environments(old: &Path, new: &Path, environments: &[PathBuf]) -> Vec<String> {
    let mut problems = Vec::new();
    // uv's "3.12" shortcuts (symlinks; junctions on Windows) lead into the old folder. Remove them,
    // noting which Python each led to: environments are pointed straight at it instead, and uv
    // makes the shortcut again the next time it installs that version.
    let mut shortcuts: Vec<(OsString, OsString)> = Vec::new();
    for entry in std::fs::read_dir(new.join("runtimes").join("python")).into_iter().flatten().flatten() {
        let path = entry.path();
        let is_link = std::fs::symlink_metadata(&path).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        if !is_link {
            continue;
        }
        if let Some(target) = std::fs::read_link(&path).ok().and_then(|t| t.file_name().map(|n| n.to_owned())) {
            shortcuts.push((entry.file_name(), target));
        }
        if std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir(&path)).is_err() {
            problems.push(format!("Could not remove the Python shortcut {}", path.display()));
        }
    }
    let relocate = |path: &Path| -> PathBuf {
        let moved = match strip_root(path, old) {
            Some(rest) => new.join(rest),
            None => path.to_path_buf(),
        };
        moved
            .components()
            .map(|part| {
                let name = part.as_os_str();
                shortcuts.iter().find(|(link, _)| link == name).map_or(name, |(_, target)| target.as_os_str())
            })
            .collect()
    };
    for env in environments {
        let config = env.join("pyvenv.cfg");
        let Ok(text) = std::fs::read_to_string(&config) else { continue };
        let fixed: Vec<String> = text
            .lines()
            .map(|line| match line.split_once('=') {
                Some((key, value)) if key.trim() == "home" => {
                    format!("home = {}", relocate(Path::new(value.trim())).display())
                }
                _ => line.to_string(),
            })
            .collect();
        if std::fs::write(&config, fixed.join("\n") + "\n").is_err() {
            problems.push(format!("Could not update {}", config.display()));
        }
        #[cfg(unix)]
        for entry in std::fs::read_dir(env.join("bin")).into_iter().flatten().flatten() {
            let link = entry.path();
            let Ok(target) = std::fs::read_link(&link) else { continue };
            if target.is_absolute() {
                let _ = std::fs::remove_file(&link);
                if std::os::unix::fs::symlink(relocate(&target), &link).is_err() {
                    problems.push(format!("Could not update {}", link.display()));
                }
            }
        }
    }
    problems
}

/// `path` without the leading `root`, also when one of them is written differently: Windows may
/// give the same folder as a short (RUNNER~1) or long name, in another case, or with a \\?\ prefix.
fn strip_root(path: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(rest) = path.strip_prefix(root) {
        return Some(rest.to_path_buf());
    }
    let mut roots = vec![root.to_path_buf()];
    if let (Some(parent), Some(name)) = (root.parent(), root.file_name()) {
        if let Ok(parent) = std::fs::canonicalize(parent) {
            roots.push(parent.join(name));
        }
    }
    let plain = |p: &Path| {
        let text = p.to_string_lossy().replace('/', "\\");
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_string();
        if cfg!(windows) { text.to_lowercase() } else { text }
    };
    let path_text = plain(path);
    for root in roots {
        let root_text = plain(&root);
        if let Some(rest) = path_text.strip_prefix(&root_text) {
            if rest.is_empty() || rest.starts_with('\\') {
                // Take the rest from the original path, to keep its case.
                let original = path.to_string_lossy().replace('/', "\\");
                let tail = &original[original.len() - rest.len()..];
                return Some(PathBuf::from(tail.trim_start_matches('\\').replace('\\', std::path::MAIN_SEPARATOR_STR)));
            }
        }
    }
    None
}

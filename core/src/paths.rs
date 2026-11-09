//! Where things are kept, and path safety checks.

use std::path::{Component, Path, PathBuf};

/// The data folder's name. It dates from when an older Python version of the hub used "Toki Hub";
/// it stays the same so existing settings and installed games carry over.
pub const APP_NAME: &str = "Toki Hub (Tauri)";

/// Settings, installed games, downloads and logs: `%LOCALAPPDATA%\Toki Hub (Tauri)` on Windows.
pub fn default_data_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("AppData").join("Local"));
    #[cfg(target_os = "macos")]
    let base = home().join("Library").join("Application Support");
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"));
    base.join(APP_NAME)
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `~/...` to a full path.
pub fn expand_home(text: &str) -> PathBuf {
    match text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        Some(rest) => home().join(rest),
        None if text == "~" => home(),
        None => PathBuf::from(text),
    }
}

/// The platform name used by `builds[].platform` in the catalog.
pub fn current_platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// A string that is safe to use as a single folder or file name on every OS.
pub fn safe_name(text: &str) -> String {
    let mut out = String::new();
    let mut replacing = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || "._-".contains(c) {
            out.push(c);
            replacing = false;
        } else if !replacing {
            out.push('_');
            replacing = true;
        }
    }
    let trimmed = out.trim_matches(|c| c == '.' || c == '_');
    if trimmed.is_empty() { "_".to_string() } else { trimmed.to_string() }
}

/// A full path with `..` worked out and symlinks followed, even if the end of it doesn't exist yet.
pub fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut normal = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other.as_os_str()),
        }
    }
    let mut existing = normal.clone();
    let mut missing = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => {
                missing.push(name.to_owned());
                existing.pop();
            }
            None => break,
        }
    }
    let mut resolved = std::fs::canonicalize(&existing).unwrap_or(existing);
    for name in missing.iter().rev() {
        resolved.push(name);
    }
    resolved
}

/// True when `path` is `folder` or inside it.
pub fn is_within(path: &Path, folder: &Path) -> bool {
    resolve(path).starts_with(resolve(folder))
}

/// Find a program on the PATH, like the `which` command.
pub fn which(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .map(str::to_string)
            .chain([String::new()])
            .collect()
    } else {
        vec![String::new()]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for extension in &extensions {
            let file = dir.join(format!("{name}{extension}"));
            if file.is_file() {
                return Some(file);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names() {
        assert_eq!(safe_name("1.0"), "1.0");
        assert_eq!(safe_name("2.0 beta/3"), "2.0_beta_3");
        assert_eq!(safe_name("../.."), "_");
    }

    #[test]
    fn within() {
        let base = std::env::temp_dir().join("toki-hub-within-test");
        assert!(is_within(&base.join("a/b.txt"), &base));
        assert!(!is_within(&base.join("../escaped.txt"), &base));
        assert!(!is_within(&base.join("a/../../escaped.txt"), &base));
    }
}

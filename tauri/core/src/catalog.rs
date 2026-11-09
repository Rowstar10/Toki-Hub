//! The game catalog: one folder per game, each with a `game.json` (see catalog/README.md).
//!
//! Loading never fails for a bad entry: whatever can be used is kept, and everything wrong is
//! collected in `Catalog::problems` so the hub can show it. The rules match the Python version.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths::{is_within, safe_name};
use crate::versions::sort_newest_first;
use crate::{fail, Result};

pub const PLATFORMS: [&str; 4] = ["windows", "macos", "linux", "any"];
pub const IMAGE_TYPES: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "svg"];
const SOURCE_TYPES: [&str; 4] = ["url", "github-release", "github-archive", "local"];
const LAUNCH_TYPES: [&str; 3] = ["executable", "python", "open"];
const GAME_FIELDS: [&str; 9] = ["title", "tagline", "description", "developer", "tags", "icon", "cover", "accent", "versions"];
const VERSION_FIELDS: [&str; 4] = ["version", "date", "notes", "builds"];
const BUILD_FIELDS: [&str; 4] = ["platform", "source", "launch", "note"];
const LAUNCH_FIELDS: [&str; 3] = ["type", "path", "args"];

/// Where to download a build from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Source {
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        sha256: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        filename: String,
    },
    GithubRelease {
        repo: String,
        tag: String,
        asset: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        sha256: String,
    },
    GithubArchive {
        repo: String,
        #[serde(rename = "ref")]
        reference: String,
    },
    Local {
        path: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        sha256: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchKind {
    Executable,
    Python,
    Open,
}

/// How to start a build: `path` is relative to the installed version's folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Launch {
    #[serde(rename = "type")]
    pub kind: LaunchKind,
    pub path: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Build {
    pub platform: String,
    pub source: Source,
    pub launch: Launch,
    pub note: String,
}

impl Build {
    pub fn source_label(&self) -> String {
        match &self.source {
            Source::GithubRelease { repo, tag, .. } => format!("GitHub release {repo} @ {tag}"),
            Source::GithubArchive { repo, reference } => {
                let short = if reference.len() == 40 { &reference[..10] } else { reference };
                format!("GitHub source {repo} @ {short}")
            }
            Source::Local { path, .. } => format!("Local files: {path}"),
            Source::Url { url, .. } => url.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Version {
    pub version: String,
    pub date: String,
    pub notes: String,
    pub builds: Vec<Build>,
}

impl Version {
    /// The installed version's folder name.
    pub fn folder(&self) -> String {
        safe_name(&self.version)
    }

    /// The build to use on `platform`: an exact match first, otherwise an "any" build.
    pub fn build_for(&self, platform: &str) -> Option<&Build> {
        [platform, "any"]
            .iter()
            .find_map(|wanted| self.builds.iter().find(|build| build.platform == *wanted))
    }
}

#[derive(Debug, Clone)]
pub struct Game {
    pub id: String,
    pub folder: PathBuf,
    pub title: String,
    pub tagline: String,
    pub description: String,
    pub developer: String,
    pub tags: Vec<String>,
    pub accent: String,
    pub icon: Option<PathBuf>,
    pub cover: Option<PathBuf>,
    /// Newest first.
    pub versions: Vec<Version>,
}

impl Game {
    pub fn version(&self, name: &str) -> Result<&Version> {
        match self.versions.iter().find(|v| v.version == name) {
            Some(version) => Ok(version),
            None => fail(format!("{} has no version '{name}'.", self.title)),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub folders: Vec<PathBuf>,
    /// Ordered by title.
    pub games: Vec<Game>,
    pub problems: Vec<String>,
}

impl Catalog {
    /// Read catalog folders in order: a game in a later folder replaces one with the same id in an
    /// earlier folder. Folders that do not exist are skipped.
    pub fn load(folders: &[PathBuf]) -> Catalog {
        let mut catalog = Catalog { folders: folders.to_vec(), ..Default::default() };
        let existing: Vec<&PathBuf> = folders.iter().filter(|f| f.is_dir()).collect();
        if existing.is_empty() {
            let last = folders.last().map(|f| f.display().to_string()).unwrap_or_default();
            catalog.problems.push(format!("Catalog folder not found: {last}"));
            return catalog;
        }
        let mut games: Vec<Game> = Vec::new();
        for folder in existing {
            let Ok(entries) = std::fs::read_dir(folder) else { continue };
            let mut dirs: Vec<PathBuf> = entries
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| path.is_dir())
                .filter(|path| !file_name(path).starts_with(['.', '_']))
                .collect();
            dirs.sort();
            for dir in dirs {
                if let Some(game) = load_game(&dir, &mut catalog.problems) {
                    games.retain(|existing| existing.id != game.id);
                    games.push(game);
                }
            }
        }
        games.sort_by_key(|game| game.title.to_lowercase());
        catalog.games = games;
        catalog
    }

    pub fn get(&self, id: &str) -> Option<&Game> {
        self.games.iter().find(|game| game.id == id)
    }

    pub fn game(&self, id: &str) -> Result<&Game> {
        match self.get(id) {
            Some(game) => Ok(game),
            None => fail(format!("There is no game '{id}' in the catalog.")),
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Collects problems, each prefixed with where in game.json it was found.
struct Checker<'a> {
    prefix: String,
    problems: &'a mut Vec<String>,
}

impl Checker<'_> {
    fn report(&mut self, place: &str, message: &str) {
        let separator = if place.is_empty() { String::new() } else { format!("{place}: ") };
        self.problems.push(format!("{}: {separator}{message}", self.prefix));
    }

    fn text(&mut self, object: &Map<String, Value>, key: &str, place: &str, required: bool) -> String {
        match object.get(key) {
            None | Some(Value::Null) => {
                if required {
                    self.report(place, &format!("'{key}' is required"));
                }
                String::new()
            }
            Some(Value::String(text)) if !(required && text.trim().is_empty()) => text.trim().to_string(),
            Some(_) => {
                let kind = if required { "non-empty text" } else { "text" };
                self.report(place, &format!("'{key}' must be {kind}"));
                String::new()
            }
        }
    }

    fn unknown(&mut self, object: &Map<String, Value>, allowed: &[&str], place: &str) {
        for key in object.keys() {
            if !allowed.contains(&key.as_str()) && !key.starts_with('_') {
                self.report(place, &format!("unknown field '{key}' (ignored; a typo?)"));
            }
        }
    }
}

fn is_game_id(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_repo(text: &str) -> bool {
    let part = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
    matches!(text.split_once('/'), Some((owner, name)) if part(owner) && part(name))
}

fn is_colour(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn load_game(dir: &Path, problems: &mut Vec<String>) -> Option<Game> {
    let id = file_name(dir);
    let mut check = Checker { prefix: format!("{id}/game.json"), problems };
    if !is_game_id(&id) {
        check.report("", "folder name must be lowercase letters, digits and dashes (it is the game's id)");
        return None;
    }
    let path = dir.join("game.json");
    if !path.is_file() {
        check.report("", "file is missing");
        return None;
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            check.report("", &format!("cannot be read: {error}"));
            return None;
        }
    };
    let data: Value = match serde_json::from_str(&text) {
        Ok(data) => data,
        Err(error) => {
            check.report("", &format!("is not valid JSON ({error})"));
            return None;
        }
    };
    let Value::Object(data) = data else {
        check.report("", "must contain a JSON object { ... }");
        return None;
    };

    check.unknown(&data, &GAME_FIELDS, "");
    let title = check.text(&data, "title", "", true);
    if title.is_empty() {
        return None;
    }
    let tags = match data.get("tags") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(|t| t.as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string))
            .collect(),
        Some(_) => {
            check.report("", "'tags' must be a list of text");
            Vec::new()
        }
    };
    let mut accent = check.text(&data, "accent", "", false);
    if !accent.is_empty() && !is_colour(&accent) {
        check.report("", "'accent' must be a colour like \"#3fb0ef\"");
        accent.clear();
    }
    let mut game = Game {
        id: id.clone(),
        folder: dir.to_path_buf(),
        tagline: check.text(&data, "tagline", "", false),
        description: check.text(&data, "description", "", false),
        developer: check.text(&data, "developer", "", false),
        tags,
        accent,
        icon: image(&data, "icon", dir, &mut check),
        cover: image(&data, "cover", dir, &mut check),
        title,
        versions: Vec::new(),
    };

    let empty = Vec::new();
    let versions = match data.get("versions") {
        None => &empty,
        Some(Value::Array(items)) => items,
        Some(_) => {
            check.report("", "'versions' must be a list");
            &empty
        }
    };
    let mut seen: Vec<(String, String)> = Vec::new();
    for (i, entry) in versions.iter().enumerate() {
        let place = format!("versions[{i}]");
        let Some(version) = load_version(entry, &place, &mut check) else { continue };
        if let Some((_, other)) = seen.iter().find(|(folder, _)| *folder == version.folder()) {
            let message = format!("version '{}' clashes with '{other}'", version.version);
            check.report(&place, &message);
            continue;
        }
        seen.push((version.folder(), version.version.clone()));
        game.versions.push(version);
    }
    sort_newest_first(&mut game.versions, |v| &v.version);
    Some(game)
}

fn image(data: &Map<String, Value>, key: &str, dir: &Path, check: &mut Checker) -> Option<PathBuf> {
    let name = check.text(data, key, "", false);
    if name.is_empty() {
        return None;
    }
    let path = dir.join(&name);
    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !is_within(&path, dir) || !IMAGE_TYPES.contains(&extension.as_str()) {
        check.report("", &format!("'{key}' must be an image file inside the game's folder"));
        return None;
    }
    if !path.is_file() {
        check.report("", &format!("'{key}' file not found: {name}"));
        return None;
    }
    Some(path)
}

fn load_version(entry: &Value, place: &str, check: &mut Checker) -> Option<Version> {
    let Value::Object(entry) = entry else {
        check.report(place, "must be an object { ... }");
        return None;
    };
    check.unknown(entry, &VERSION_FIELDS, place);
    let name = check.text(entry, "version", place, true);
    if name.is_empty() {
        return None;
    }
    if name.chars().count() > 64 {
        check.report(place, "'version' is too long (64 characters at most)");
        return None;
    }
    let place = format!("version {name}");
    let mut version = Version {
        date: check.text(entry, "date", &place, false),
        notes: check.text(entry, "notes", &place, false),
        version: name,
        builds: Vec::new(),
    };
    let empty = Vec::new();
    let builds = match entry.get("builds") {
        None => &empty,
        Some(Value::Array(items)) => items,
        Some(_) => {
            check.report(&place, "'builds' must be a list");
            &empty
        }
    };
    for (i, build) in builds.iter().enumerate() {
        if let Some(build) = load_build(build, &format!("{place}: builds[{i}]"), check) {
            version.builds.push(build);
        }
    }
    Some(version)
}

fn load_build(entry: &Value, place: &str, check: &mut Checker) -> Option<Build> {
    let Value::Object(entry) = entry else {
        check.report(place, "must be an object { ... }");
        return None;
    };
    check.unknown(entry, &BUILD_FIELDS, place);
    let platform = match entry.get("platform") {
        None => "any".to_string(),
        Some(Value::String(p)) if PLATFORMS.contains(&p.as_str()) => p.clone(),
        Some(_) => {
            check.report(place, &format!("'platform' must be one of {}", PLATFORMS.join(", ")));
            return None;
        }
    };
    let source = load_source(entry.get("source"), place, check);
    let launch = load_launch(entry.get("launch"), place, check);
    let note = check.text(entry, "note", place, false);
    Some(Build { platform, source: source?, launch: launch?, note })
}

fn load_source(source: Option<&Value>, place: &str, check: &mut Checker) -> Option<Source> {
    let place = format!("{place}: source");
    let Some(Value::Object(source)) = source else {
        check.report(&place, "is required and must be an object { \"type\": ... }");
        return None;
    };
    let kind = source.get("type").and_then(Value::as_str).unwrap_or_default();
    let (required, optional): (&[&str], &[&str]) = match kind {
        "url" => (&["url"], &["sha256", "filename"]),
        "github-release" => (&["repo", "tag", "asset"], &["sha256"]),
        "github-archive" => (&["repo", "ref"], &[]),
        "local" => (&["path"], &["sha256"]),
        _ => {
            check.report(&place, &format!("'type' must be one of {}", SOURCE_TYPES.join(", ")));
            return None;
        }
    };
    let allowed: Vec<&str> = ["type"].iter().chain(required).chain(optional).copied().collect();
    check.unknown(source, &allowed, &place);
    let mut fields = Map::new();
    fields.insert("type".into(), Value::String(kind.into()));
    for key in required {
        let value = check.text(source, key, &place, true);
        if value.is_empty() {
            return None;
        }
        fields.insert(key.to_string(), Value::String(value));
    }
    for key in optional {
        let value = check.text(source, key, &place, false);
        if !value.is_empty() {
            fields.insert(key.to_string(), Value::String(value));
        }
    }
    let field = |fields: &Map<String, Value>, key: &str| fields.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    if fields.contains_key("repo") && !is_repo(&field(&fields, "repo")) {
        check.report(&place, "'repo' must look like \"Owner/RepoName\"");
        return None;
    }
    if kind == "url" && !field(&fields, "url").starts_with("https://") && !field(&fields, "url").starts_with("http://") {
        check.report(&place, "'url' must start with https:// or http://");
        return None;
    }
    if fields.contains_key("sha256") {
        if !is_sha256(&field(&fields, "sha256")) {
            check.report(&place, "'sha256' must be 64 hexadecimal characters");
            return None;
        }
        let lowered = field(&fields, "sha256").to_lowercase();
        fields.insert("sha256".into(), Value::String(lowered));
    }
    if fields.contains_key("filename") && safe_name(&field(&fields, "filename")) != field(&fields, "filename") {
        check.report(&place, "'filename' may only use letters, digits, '.', '_' and '-'");
        return None;
    }
    serde_json::from_value(Value::Object(fields)).ok()
}

fn load_launch(launch: Option<&Value>, place: &str, check: &mut Checker) -> Option<Launch> {
    let place = format!("{place}: launch");
    let Some(Value::Object(launch)) = launch else {
        check.report(&place, "is required and must be an object { \"type\": ..., \"path\": ... }");
        return None;
    };
    check.unknown(launch, &LAUNCH_FIELDS, &place);
    let kind = launch.get("type").and_then(Value::as_str).unwrap_or_default();
    if !LAUNCH_TYPES.contains(&kind) {
        check.report(&place, &format!("'type' must be one of {}", LAUNCH_TYPES.join(", ")));
        return None;
    }
    let path = check.text(launch, "path", &place, true);
    if path.is_empty() {
        return None;
    }
    let unix_style = path.replace('\\', "/");
    let drive = path.len() >= 2 && path.as_bytes()[0].is_ascii_alphabetic() && path.as_bytes()[1] == b':';
    if unix_style.starts_with('/') || drive || unix_style.split('/').any(|part| part == "..") {
        check.report(&place, "'path' must be relative to the installed game's folder");
        return None;
    }
    let args = match launch.get("args") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            items.iter().filter_map(|a| a.as_str().map(str::to_string)).collect()
        }
        Some(_) => {
            check.report(&place, "'args' must be a list of text");
            return None;
        }
    };
    let kind = match kind {
        "executable" => LaunchKind::Executable,
        "python" => LaunchKind::Python,
        _ => LaunchKind::Open,
    };
    Some(Launch { kind, path: unix_style, args })
}

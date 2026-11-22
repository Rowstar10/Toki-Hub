//! Games published as GitHub releases (a game's "releases" in game.json; see catalog/README.md):
//! each release of the game's public repository is a version, and its files are that version's
//! builds, so the game's source can stay private.
//!
//! The list comes from GitHub's public API, without signing in. Signed out, GitHub allows 60 calls
//! an hour, so the list is asked for with the ETag of the last answer: "nothing new" (304) doesn't
//! count against that. The last list is kept on disk, so the versions still show with no internet
//! connection or once the limit is reached. Downloads use the releases' public links, which aren't
//! counted either, and are checked against the sha256 GitHub publishes for each file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::catalog::{fill_version, Build, Releases, Source, Version};
use crate::installer::utc_now;
use crate::versions::sort_newest_first;

pub type TokenGetter = Box<dyn Fn() -> String + Send + Sync>;

/// What the hub knows of a game's releases.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listing {
    pub releases: Vec<Value>,
    /// When GitHub last answered (UTC, ISO 8601); empty if it never has.
    pub checked: String,
    /// "online" (checked this session), "saved" (the copy from last time: checking now failed or
    /// hasn't happened yet) or "none".
    pub from: String,
    /// Why the last check failed, for the player.
    pub problem: String,
}

impl Listing {
    pub fn to_json(&self) -> Value {
        json!({"from": self.from, "checked": self.checked, "problem": self.problem})
    }
}

pub struct ReleaseLists {
    folder: PathBuf,
    agent: ureq::Agent,
    api: String,
    token: TokenGetter,
    known: Mutex<HashMap<String, Listing>>,
}

impl ReleaseLists {
    /// `folder` keeps the last list of each game's releases.
    pub fn new(folder: PathBuf, agent: ureq::Agent, api: &str, token: TokenGetter) -> ReleaseLists {
        ReleaseLists { folder, agent, api: api.trim_end_matches('/').to_string(), token, known: Mutex::default() }
    }

    /// The list known now, without asking GitHub: this session's, else the one kept on disk.
    pub fn current(&self, game_id: &str) -> Listing {
        if let Some(listing) = self.known.lock().unwrap().get(game_id) {
            return listing.clone();
        }
        match self.saved(game_id) {
            Some((releases, checked, _)) => Listing { releases, checked, from: "saved".into(), problem: String::new() },
            None => Listing { from: "none".into(), ..Default::default() },
        }
    }

    /// Ask GitHub for the game's releases. If that fails, the last list stays, with why.
    pub fn refresh(&self, game_id: &str, source: &Releases) -> Listing {
        let saved = self.saved(game_id);
        let etag = saved.as_ref().map(|(_, _, etag)| etag.clone()).unwrap_or_default();
        let listing = match self.fetch(source, &etag) {
            Ok(Some((releases, etag))) => {
                self.keep(game_id, &releases, &etag);
                Listing { releases, checked: utc_now(), from: "online".into(), problem: String::new() }
            }
            Ok(None) => {
                // Nothing new since last time
                let (releases, _, etag) = saved.unwrap_or_default();
                self.keep(game_id, &releases, &etag);
                Listing { releases, checked: utc_now(), from: "online".into(), problem: String::new() }
            }
            Err(problem) => match saved {
                Some((releases, checked, _)) => Listing { releases, checked, from: "saved".into(), problem },
                None => Listing { from: "none".into(), problem, ..Default::default() },
            },
        };
        self.known.lock().unwrap().insert(game_id.to_string(), listing.clone());
        listing
    }

    /// Every release (Ok(None) when the first page is unchanged since `etag`).
    fn fetch(&self, source: &Releases, etag: &str) -> Result<Option<(Vec<Value>, String)>, String> {
        let mut url = format!("{}/repos/{}/releases?per_page=100", self.api, source.repo);
        let mut releases = Vec::new();
        let mut first_etag = String::new();
        for page in 0..10 {
            let mut request = self
                .agent
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
            if page == 0 && !etag.is_empty() {
                request = request.header("If-None-Match", etag);
            }
            let token = (self.token)();
            if !token.is_empty() {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
            let mut response = request
                .call()
                .map_err(|_| "Couldn't check for new versions: is the internet connected?".to_string())?;
            let status = response.status().as_u16();
            if page == 0 && status == 304 {
                return Ok(None);
            }
            if !(200..300).contains(&status) {
                return Err(problem(status, &response));
            }
            if page == 0 {
                first_etag = header(&response, "etag");
            }
            let next = next_page(&header(&response, "link"));
            let text = response
                .body_mut()
                .with_config()
                .limit(64 * 1024 * 1024)
                .read_to_string()
                .map_err(|_| "Couldn't check for new versions: the answer was cut off".to_string())?;
            let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else {
                return Err("Couldn't check for new versions: GitHub's answer couldn't be read".into());
            };
            releases.extend(items.iter().map(slim));
            match next {
                Some(next) => url = next,
                None => break,
            }
        }
        Ok(Some((releases, first_etag)))
    }

    fn file(&self, game_id: &str) -> PathBuf {
        self.folder.join(format!("{game_id}.json"))
    }

    /// (releases, when checked, ETag) from disk.
    fn saved(&self, game_id: &str) -> Option<(Vec<Value>, String, String)> {
        let text = std::fs::read_to_string(self.file(game_id)).ok()?;
        let saved: Value = serde_json::from_str(&text).ok()?;
        let releases = saved["releases"].as_array()?.clone();
        let text = |key: &str| saved[key].as_str().unwrap_or_default().to_string();
        Some((releases, text("checked"), text("etag")))
    }

    fn keep(&self, game_id: &str, releases: &[Value], etag: &str) {
        let path = self.file(game_id);
        let partial = path.with_extension("tmp");
        let body = json!({"checked": utc_now(), "etag": etag, "releases": releases});
        let kept = std::fs::create_dir_all(&self.folder)
            .and_then(|_| std::fs::write(&partial, body.to_string()))
            .and_then(|_| std::fs::rename(&partial, &path));
        if kept.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
    }
}

/// Just what the hub needs of a release.
fn slim(release: &Value) -> Value {
    let assets: Vec<Value> = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|asset| json!({"name": asset["name"], "size": asset["size"], "digest": asset["digest"]}))
        .collect();
    json!({
        "tag_name": release["tag_name"],
        "draft": release["draft"],
        "prerelease": release["prerelease"],
        "published_at": release["published_at"],
        "body": release["body"],
        "assets": assets,
    })
}

fn header(response: &ureq::http::Response<ureq::Body>, name: &str) -> String {
    response.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string()
}

/// The `rel="next"` address in a Link header.
fn next_page(link: &str) -> Option<String> {
    link.split(',').find(|part| part.contains("rel=\"next\"")).and_then(|part| {
        let start = part.find('<')? + 1;
        let end = part.find('>')?;
        Some(part[start..end].to_string())
    })
}

/// What went wrong, in words for the player (no addresses or account names).
fn problem(status: u16, response: &ureq::http::Response<ureq::Body>) -> String {
    let remaining = header(response, "x-ratelimit-remaining");
    if matches!(status, 403 | 429) && (remaining == "0" || status == 429) {
        let reset: u64 = header(response, "x-ratelimit-reset").parse().unwrap_or(0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let minutes = if reset > now { (reset - now).div_ceil(60) } else { 60 };
        return format!(
            "GitHub's limit for checking new versions was reached: try again in about {minutes} minute{}.",
            if minutes == 1 { "" } else { "s" }
        );
    }
    if status == 404 {
        return "Couldn't find this game's releases.".into();
    }
    format!("Couldn't check for new versions (GitHub answered {status}).")
}

/// The versions in a game's releases, newest first. A release is a version when its tag fits the
/// game's pattern and it has at least one of the game's build files.
pub fn versions(source: &Releases, releases: &[Value]) -> Vec<Version> {
    let (prefix, suffix) = source.tag.split_once("{version}").unwrap_or(("v", ""));
    let mut found: Vec<Version> = Vec::new();
    for release in releases {
        if release["draft"].as_bool() == Some(true) {
            continue;
        }
        let tag = release["tag_name"].as_str().unwrap_or_default();
        let Some(version) = tag.strip_prefix(prefix).and_then(|rest| rest.strip_suffix(suffix)) else { continue };
        if version.is_empty() || version.len() > 64 || version.contains(['/', '\\']) || found.iter().any(|v| v.version == version) {
            continue;
        }
        let assets = release["assets"].as_array().cloned().unwrap_or_default();
        let builds: Vec<Build> = source
            .builds
            .iter()
            .filter_map(|build| {
                let name = fill_version(&build.asset, version);
                let asset = assets.iter().find(|asset| asset["name"].as_str() == Some(name.as_str()))?;
                let sha256 = asset["digest"]
                    .as_str()
                    .and_then(|digest| digest.strip_prefix("sha256:"))
                    .filter(|hex| hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
                    .unwrap_or_default()
                    .to_lowercase();
                let mut launch = build.launch.clone();
                launch.path = fill_version(&launch.path, version);
                launch.package = fill_version(&launch.package, version);
                Some(Build {
                    platform: build.platform.clone(),
                    source: Source::GithubRelease { repo: source.repo.clone(), tag: tag.to_string(), asset: name, sha256 },
                    launch,
                    note: fill_version(&build.note, version),
                })
            })
            .collect();
        if builds.is_empty() {
            continue;
        }
        let details = release_notes(release["body"].as_str().unwrap_or_default());
        let date = release["published_at"].as_str().unwrap_or_default().chars().take(10).collect();
        found.push(Version { version: version.to_string(), date, notes: summary(&details), details, builds });
    }
    sort_newest_first(&mut found, |v| &v.version);
    found
}

/// A release's notes, without what's after a `---` line (how to download it, for GitHub's page).
fn release_notes(body: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let notes: Vec<&str> = body.lines().take_while(|line| line.trim() != "---").collect();
    notes.join("\n").trim().to_string()
}

/// The first paragraph of the notes: the quick summary, with "Show more" for the rest.
fn summary(details: &str) -> String {
    let first: Vec<&str> = details
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .take_while(|line| !line.trim().is_empty() && !line.starts_with('#') && !line.trim_start().starts_with("- "))
        .collect();
    let first = first.join("\n").trim().to_string();
    if first.is_empty() { details.to_string() } else { first }
}

/// A game's versions: the ones listed in game.json, with its releases added. A release's builds
/// replace listed builds for the same platform; a listed version keeps its own summary, and the
/// release's notes become its full notes.
pub fn merge(listed: &[Version], released: Vec<Version>) -> Vec<Version> {
    let mut versions = listed.to_vec();
    for release in released {
        match versions.iter_mut().find(|v| v.version == release.version) {
            Some(version) => {
                version.builds.retain(|b| !release.builds.iter().any(|r| r.platform == b.platform));
                let mut builds = release.builds;
                builds.append(&mut version.builds);
                version.builds = builds;
                if version.date.is_empty() {
                    version.date = release.date;
                }
                if version.notes.is_empty() {
                    version.notes = release.notes;
                }
                version.details = release.details;
            }
            None => versions.push(release),
        }
    }
    for version in &mut versions {
        if version.details.trim() == version.notes.trim() {
            version.details.clear();
        }
    }
    sort_newest_first(&mut versions, |v| &v.version);
    versions
}

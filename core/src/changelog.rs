//! Games' changelogs: a Markdown file with a `## <version>` heading for each version, fetched from
//! where the game's catalog entry says (see catalog/README.md).
//!
//! Each game's changelog is fetched once per session (again after the catalog is reloaded), and the
//! last copy that arrived is kept on disk, so the notes still show without an internet connection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Map, Value};

use crate::catalog::Game;
use crate::sources::SourceResolver;

/// Each version's section, in the file's order. A heading is `## 1.3`, `## v1.3 — 2026-11-10` or
/// `## [1.3] - 2026-11-10`: its first word, without brackets or a leading "v", is the version. Text
/// before the first `## ` heading (the file's title and introduction) belongs to no version.
pub fn sections(text: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(version) = heading_version(heading) {
                found.push((version, Vec::new()));
                continue;
            }
        }
        if let Some((_, body)) = found.last_mut() {
            body.push(line);
        }
    }
    found.into_iter().map(|(version, body)| (version, body.join("\n").trim().to_string())).collect()
}

fn heading_version(heading: &str) -> Option<String> {
    let word = heading.split_whitespace().next()?.trim_start_matches('[').trim_end_matches(']');
    let word = word.strip_prefix(['v', 'V']).unwrap_or(word);
    (!word.is_empty()).then(|| word.to_string())
}

pub struct Changelogs {
    folder: PathBuf,
    loaded: Mutex<HashMap<String, Value>>,
}

impl Changelogs {
    /// `folder` keeps the last copy of each game's changelog.
    pub fn new(folder: PathBuf) -> Changelogs {
        Changelogs { folder, loaded: Mutex::default() }
    }

    /// Fetch every changelog again the next time it is asked for.
    pub fn forget(&self) {
        self.loaded.lock().unwrap().clear();
    }

    /// For the UI: `{"versions": {"1.3": "...", ...}, "from": "online" | "saved" | "none", "error": "..."}`.
    /// "saved" is the copy kept from last time, used when the changelog can't be fetched now.
    pub fn get(&self, game: &Game, resolver: &SourceResolver) -> Value {
        let Some(source) = &game.changelog else {
            return answer("", "none", "");
        };
        if let Some(known) = self.loaded.lock().unwrap().get(&game.id) {
            return known.clone();
        }
        let saved = self.folder.join(format!("{}.md", game.id));
        let result = match resolver.fetch_text(source) {
            Ok(text) => {
                self.keep(&saved, &text);
                answer(&text, "online", "")
            }
            Err(error) => match std::fs::read_to_string(&saved) {
                Ok(text) => answer(&text, "saved", &error.0),
                Err(_) => answer("", "none", &error.0),
            },
        };
        self.loaded.lock().unwrap().insert(game.id.clone(), result.clone());
        result
    }

    fn keep(&self, path: &Path, text: &str) {
        let partial = path.with_extension("tmp");
        let kept = std::fs::create_dir_all(&self.folder)
            .and_then(|_| std::fs::write(&partial, text))
            .and_then(|_| std::fs::rename(&partial, path));
        if kept.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
    }
}

fn answer(text: &str, from: &str, error: &str) -> Value {
    let mut versions = Map::new();
    for (version, body) in sections(text) {
        versions.entry(version).or_insert(Value::String(body));
    }
    json!({"versions": versions, "from": from, "error": error})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_are_split_at_version_headings() {
        let text = "# Changelog\r\n\r\nIntro text.\r\n\r\n## 1.3 — 2026-11-10\r\n\r\nSummary.\r\n\r\n### Added\r\n- One\r\n- Two\r\n\r\n\
                    ## [v1.2] - 2026-11-04\n- Older\n\n## 0.1\n";
        assert_eq!(
            sections(text),
            vec![
                ("1.3".to_string(), "Summary.\n\n### Added\n- One\n- Two".to_string()),
                ("1.2".to_string(), "- Older".to_string()),
                ("0.1".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn no_headings_means_no_sections() {
        assert!(sections("Just some text\n### Added\n- x").is_empty());
        assert!(sections("").is_empty());
    }

    #[test]
    fn the_first_section_for_a_version_wins() {
        let found = answer("## 1.0\nFirst\n## 1.0\nSecond", "online", "");
        assert_eq!(found["versions"]["1.0"], "First");
    }
}

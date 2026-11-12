//! Turning a build's `source` into something the installer can fetch.
//!
//! GitHub sources need no token when the repository is public: they are downloaded from GitHub's
//! ordinary download links, which have no rate limit. Once a token is set in Settings (or the
//! GITHUB_TOKEN environment variable), they go through the GitHub API instead, which also works for
//! private repositories. The token is never sent on to a redirect's host: ureq drops the
//! Authorization header on every redirect.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::catalog::{Changelog, Source};
use crate::paths::{expand_home, safe_name};
use crate::{fail, HubError, Result};

pub const GITHUB_API: &str = "https://api.github.com";
/// GitHub's website, for downloads from public repositories without a token.
pub const GITHUB_WEB: &str = "https://github.com";
pub const USER_AGENT: &str = concat!("TokiHub/", env!("CARGO_PKG_VERSION"));

pub enum Plan {
    Download(Download),
    Local(LocalFiles),
}

pub struct Download {
    pub url: String,
    pub filename: String,
    pub headers: Vec<(&'static str, String)>,
    /// Set for GitHub downloads, for error messages.
    pub github_what: String,
    pub sha256: String,
}

pub struct LocalFiles {
    pub path: PathBuf,
    pub sha256: String,
}

type TokenGetter = Box<dyn Fn() -> String + Send + Sync>;

pub struct SourceResolver {
    token: TokenGetter,
    github_api: String,
    github_web: String,
    agent: ureq::Agent,
}

/// The HTTP client: errors like 404 come back as responses, stalled connections time out, and
/// certificates are checked the way the operating system checks them.
pub fn http_agent() -> ureq::Agent {
    // On Android the system's certificate check needs Java set up for it; the built-in list of
    // certificate authorities (as browsers ship) does the same job there.
    let roots = if cfg!(target_os = "android") { ureq::tls::RootCerts::WebPki } else { ureq::tls::RootCerts::PlatformVerifier };
    let tls = ureq::tls::TlsConfig::builder().root_certs(roots).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .user_agent(USER_AGENT)
        .max_redirects(10)
        .build()
        .into()
}

impl SourceResolver {
    pub fn new(token: TokenGetter, github_api: &str, github_web: &str) -> SourceResolver {
        SourceResolver {
            token,
            github_api: github_api.trim_end_matches('/').to_string(),
            github_web: github_web.trim_end_matches('/').to_string(),
            agent: http_agent(),
        }
    }

    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    fn auth(&self) -> Vec<(&'static str, String)> {
        let token = (self.token)();
        if token.is_empty() { Vec::new() } else { vec![("Authorization", format!("Bearer {token}"))] }
    }

    pub fn resolve(&self, source: &Source, game_folder: &Path) -> Result<Plan> {
        match source {
            Source::Url { url, sha256, filename } => Ok(Plan::Download(Download {
                url: url.clone(),
                filename: if filename.is_empty() { filename_from_url(url) } else { filename.clone() },
                headers: Vec::new(),
                github_what: String::new(),
                sha256: sha256.clone(),
            })),
            Source::Local { path, sha256 } => {
                let mut path = expand_home(path);
                if !path.is_absolute() {
                    path = game_folder.join(path);
                }
                if !path.exists() {
                    return fail(format!("The local files for this build are not on this computer: {}", path.display()));
                }
                Ok(Plan::Local(LocalFiles { path, sha256: sha256.clone() }))
            }
            Source::GithubArchive { repo, reference } => {
                let auth = self.auth();
                let (url, headers) = if auth.is_empty() {
                    (format!("{}/{repo}/archive/{}.zip", self.github_web, quote(reference)), Vec::new())
                } else {
                    let mut headers = vec![("Accept", "application/vnd.github+json".to_string())];
                    headers.extend(auth);
                    (format!("{}/repos/{repo}/zipball/{}", self.github_api, quote(reference)), headers)
                };
                let name = repo.split('/').nth(1).unwrap_or(repo);
                Ok(Plan::Download(Download {
                    url,
                    filename: format!("{name}-{}.zip", short_ref(reference)),
                    headers,
                    github_what: format!("source code of {repo} at '{reference}'"),
                    sha256: String::new(),
                }))
            }
            Source::GithubRelease { repo, tag, asset, sha256 } => {
                if self.auth().is_empty() {
                    Ok(Plan::Download(Download {
                        url: format!("{}/{repo}/releases/download/{}/{}", self.github_web, quote(tag), quote(asset)),
                        filename: asset.clone(),
                        headers: Vec::new(),
                        github_what: format!("file '{asset}' in release '{tag}' of {repo}"),
                        sha256: sha256.clone(),
                    }))
                } else {
                    self.release_asset(repo, tag, asset, sha256)
                }
            }
        }
    }

    fn release_asset(&self, repo: &str, tag: &str, asset_name: &str, sha256: &str) -> Result<Plan> {
        let url = format!("{}/repos/{repo}/releases/tags/{}", self.github_api, quote(tag));
        let mut request = self.agent.get(&url).header("Accept", "application/vnd.github+json");
        for (key, value) in self.auth() {
            request = request.header(key, value);
        }
        let mut response = request
            .call()
            .map_err(|e| HubError(format!("Could not reach GitHub: {e}")))?;
        if !response.status().is_success() {
            return Err(github_error(&response, &format!("release '{tag}' of {repo}")));
        }
        let release: Value = response
            .body_mut()
            .read_to_string()
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or_else(|| HubError("GitHub sent an answer Toki Hub could not read.".into()))?;
        let assets = release.get("assets").and_then(Value::as_array).cloned().unwrap_or_default();
        for asset in &assets {
            if asset.get("name").and_then(Value::as_str) == Some(asset_name) {
                let mut headers = vec![("Accept", "application/octet-stream".to_string())];
                headers.extend(self.auth());
                return Ok(Plan::Download(Download {
                    url: asset.get("url").and_then(Value::as_str).unwrap_or_default().to_string(),
                    filename: asset_name.to_string(),
                    headers,
                    github_what: format!("file '{asset_name}' in release '{tag}' of {repo}"),
                    sha256: sha256.to_string(),
                }));
            }
        }
        let names: Vec<&str> = assets.iter().filter_map(|a| a.get("name").and_then(Value::as_str)).collect();
        let names = if names.is_empty() { "none".to_string() } else { names.join(", ") };
        fail(format!("Release '{tag}' of {repo} has no file named '{asset_name}' (it has: {names})."))
    }

    /// A small text file, such as a game's changelog.
    pub fn fetch_text(&self, source: &Changelog) -> Result<String> {
        let (url, what, github) = match source {
            Changelog::Github { repo, path, reference } => {
                let segments: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(quote).collect();
                let mut url = format!("{}/repos/{repo}/contents/{}", self.github_api, segments.join("/"));
                if !reference.is_empty() {
                    url.push_str(&format!("?ref={}", quote(reference)));
                }
                (url, format!("file '{path}' in {repo}"), true)
            }
            Changelog::Url { url } => (url.clone(), format!("changelog at {url}"), false),
        };
        let mut request = self.agent.get(&url);
        if github {
            request = request.header("Accept", "application/vnd.github.raw+json");
            for (key, value) in self.auth() {
                request = request.header(key, value);
            }
        }
        let mut response = request
            .call()
            .map_err(|e| HubError(format!("Could not fetch the {what}: {e}")))?;
        if !response.status().is_success() {
            if github {
                return Err(github_error(&response, &what));
            }
            return fail(format!("Could not fetch the {what}: HTTP {}", response.status().as_u16()));
        }
        response
            .body_mut()
            .read_to_string()
            .map_err(|e| HubError(format!("Could not read the {what}: {e}")))
    }
}

pub fn github_error(response: &ureq::http::Response<ureq::Body>, what: &str) -> HubError {
    let code = response.status().as_u16();
    let remaining = response.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok());
    let message = if matches!(code, 403 | 429) && remaining == Some("0") {
        "GitHub's rate limit was reached. Add a GitHub token in Settings, or try again later.".to_string()
    } else if code == 401 {
        "GitHub rejected the token in Settings. Check that it is correct and not expired.".to_string()
    } else if matches!(code, 403 | 404) {
        format!(
            "GitHub could not find the {what}. If the repository is private, make it public on GitHub, \
             or add a GitHub token with access to it in Settings."
        )
    } else {
        let reason = response.status().canonical_reason().unwrap_or("");
        format!("GitHub returned an error for the {what}: HTTP {code} {reason}")
    };
    HubError(message)
}

/// Percent-encode everything except unreserved characters, for one URL path segment.
pub fn quote(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Undo percent-encoding.
pub fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn filename_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let path = path.split_once("://").map(|(_, rest)| rest).unwrap_or(path);
    let name = path.split_once('/').map(|(_, rest)| rest).unwrap_or("").rsplit('/').next().unwrap_or("");
    let name = unquote(name);
    if name.is_empty() { "download".to_string() } else { safe_name(&name) }
}

fn short_ref(reference: &str) -> String {
    if reference.len() == 40 { reference[..10].to_string() } else { reference.replace('/', "-") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_from_urls() {
        assert_eq!(filename_from_url("https://example.com/files/Hu%20Taong.zip?dl=1"), "Hu_Taong.zip");
        assert_eq!(filename_from_url("https://example.com/"), "download");
        assert_eq!(filename_from_url("https://example.com"), "download");
    }

    #[test]
    fn quoting_round_trips() {
        assert_eq!(quote("v1.0/beta"), "v1.0%2Fbeta");
        assert_eq!(unquote("hu-taong%2Ficon.png"), "hu-taong/icon.png");
        assert_eq!(unquote("100%"), "100%");
    }
}

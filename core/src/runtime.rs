//! Everything a game needs besides its own files.
//!
//! For games that run from Python source, Toki Hub sets up a private Python with the game's
//! packages, using uv (https://docs.astral.sh/uv/):
//! - uv itself is downloaded once from PyPI into `<data>/tools/uv-<version>/`;
//! - Python versions are kept in `<data>/runtimes/python/`, shared by every game;
//! - each installed version gets its own environment in `<version folder>/.toki-python/`.
//!
//! Nothing is installed system-wide, nothing is added to the PATH, and no admin rights are needed.
//! Uninstalling a version deletes its environment with it.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;

use crate::download::fetch;
use crate::installer::make_executable;
use crate::launcher::read_tail;
use crate::sources::Download;
use crate::{fail, HubError, Result};

/// The uv release Toki Hub uses.
pub const UV_VERSION: &str = "0.13.5";
/// The Python a game gets when its catalog entry does not say.
pub const DEFAULT_PYTHON: &str = "3.12";
/// The environment's folder inside an installed version.
pub const ENV_DIR: &str = ".toki-python";
pub const PYPI: &str = "https://pypi.org";

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

pub struct Runtimes {
    data_dir: PathBuf,
    pypi: String,
    agent: ureq::Agent,
}

impl Runtimes {
    pub fn new(data_dir: &Path, pypi: &str, agent: ureq::Agent) -> Runtimes {
        Runtimes { data_dir: data_dir.to_path_buf(), pypi: pypi.trim_end_matches('/').to_string(), agent }
    }

    pub fn uv_path(&self) -> PathBuf {
        self.data_dir.join("tools").join(format!("uv-{UV_VERSION}")).join(format!("uv{EXE}"))
    }

    /// uv, downloaded from PyPI the first time it is needed.
    pub fn ensure_uv(&self, check_cancel: &dyn Fn() -> Result<()>) -> Result<PathBuf> {
        let target = self.uv_path();
        if target.is_file() {
            return Ok(target);
        }
        let Some(tag) = wheel_tag() else {
            return fail("Toki Hub cannot install Python on this kind of computer. Set a Python in Settings instead.");
        };
        let url = format!("{}/pypi/uv/{UV_VERSION}/json", self.pypi);
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| HubError(format!("Could not reach PyPI to get the Python installer: {e}")))?;
        if !response.status().is_success() {
            return fail(format!("PyPI answered HTTP {} for the Python installer.", response.status().as_u16()));
        }
        let release: Value = response
            .body_mut()
            .read_to_string()
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or_else(|| HubError("PyPI sent an answer Toki Hub could not read.".into()))?;
        let wheel = release["urls"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|file| file["filename"].as_str().is_some_and(|name| name.ends_with(".whl") && name.contains(tag)))
            .ok_or_else(|| HubError(format!("PyPI has no uv {UV_VERSION} for this computer ({tag}).")))?;
        let plan = Download {
            url: wheel["url"].as_str().unwrap_or_default().to_string(),
            filename: wheel["filename"].as_str().unwrap_or("uv.whl").to_string(),
            headers: Vec::new(),
            github_what: String::new(),
            sha256: wheel["digests"]["sha256"].as_str().unwrap_or_default().to_string(),
        };
        if plan.sha256.is_empty() {
            return fail("PyPI did not say what the Python installer's checksum is, so it was not used.");
        }
        let downloaded = self.data_dir.join("downloads").join(format!("{}.part", plan.filename));
        fetch(&self.agent, &plan, &downloaded, &|_, _| {}, check_cancel)?;
        let result = extract_uv(&downloaded, &target);
        let _ = std::fs::remove_file(&downloaded);
        result?;
        Ok(target)
    }

    /// Create a Python environment at `env_dir` with `packages` installed. uv's output goes to `log`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_env(
        &self,
        uv: &Path,
        env_dir: &Path,
        python: &str,
        packages: &[String],
        log: &Path,
        stage: &dyn Fn(String),
        check_cancel: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        if let Some(folder) = log.parent() {
            std::fs::create_dir_all(folder)?;
        }
        File::create(log)?;
        let failed = |what: &str| {
            let tail = read_tail(log, 6);
            let details = if tail.is_empty() { String::new() } else { format!(":\n{tail}") };
            HubError(format!("Could not {what}{details}"))
        };
        stage(format!("Setting up Python {python}…"));
        self.run_uv(uv, &["python", "install", python], log, check_cancel)
            .map_err(|e| if is_cancel(&e) { e } else { failed(&format!("install Python {python}")) })?;
        let env = env_dir.to_string_lossy().into_owned();
        self.run_uv(uv, &["venv", "--relocatable", "--no-project", "--python", python, &env], log, check_cancel)
            .map_err(|e| if is_cancel(&e) { e } else { failed("set up Python for this game") })?;
        if !packages.is_empty() {
            let names: Vec<&str> = packages.iter().map(|p| package_name(p)).collect();
            stage(format!("Installing {}…", names.join(", ")));
            let mut args = vec!["pip", "install", "--python", env.as_str()];
            args.extend(packages.iter().map(String::as_str));
            self.run_uv(uv, &args, log, check_cancel)
                .map_err(|e| if is_cancel(&e) { e } else { failed(&format!("install {}", names.join(", "))) })?;
        }
        Ok(())
    }

    fn run_uv(&self, uv: &Path, args: &[&str], log: &Path, check_cancel: &dyn Fn() -> Result<()>) -> Result<()> {
        let output = std::fs::OpenOptions::new().append(true).open(log)?;
        let mut command = Command::new(uv);
        command
            .args(args)
            .env("UV_PYTHON_INSTALL_DIR", self.data_dir.join("runtimes").join("python"))
            .env("UV_CACHE_DIR", self.data_dir.join("cache").join("uv"))
            .env("UV_PYTHON_PREFERENCE", "only-managed") // never borrow a Python from the system
            .env("UV_PYTHON_DOWNLOADS", "automatic")
            .env("UV_PYTHON_INSTALL_BIN", "0") // no python3.12 command added to the user's folder
            .env("UV_PYTHON_INSTALL_REGISTRY", "0") // nothing written to the Windows registry
            .env("UV_SYSTEM_CERTS", "1") // trust what Windows trusts, like the rest of the hub
            .env("UV_NO_CONFIG", "1")
            .env("UV_NO_PROGRESS", "1")
            .env_remove("VIRTUAL_ENV")
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(output);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console popping up
        }
        let mut child = command
            .spawn()
            .map_err(|e| HubError(format!("Could not start the Python installer: {e}")))?;
        loop {
            if let Err(cancelled) = check_cancel() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return fail(format!("uv exited with {status}")),
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => return fail(format!("Could not wait for the Python installer: {error}")),
            }
        }
    }
}

/// The Python inside an environment made by `create_env`.
pub fn env_python(env_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        env_dir.join("Scripts").join("python.exe")
    } else {
        env_dir.join("bin").join("python")
    }
}

/// "pygame==2.6.1" -> "pygame".
fn package_name(requirement: &str) -> &str {
    let end = requirement.find(|c: char| !(c.is_ascii_alphanumeric() || "-_.".contains(c))).unwrap_or(requirement.len());
    &requirement[..end]
}

fn is_cancel(error: &HubError) -> bool {
    error.0.starts_with('\u{0}')
}

/// The part of a PyPI wheel's name that says which computer it is for.
pub fn wheel_tag() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("win_amd64"),
        ("windows", "aarch64") => Some("win_arm64"),
        ("linux", "x86_64") => Some("manylinux_2_17_x86_64"),
        ("linux", "aarch64") => Some("manylinux_2_17_aarch64"),
        ("macos", "x86_64") => Some("macosx_10_12_x86_64"),
        ("macos", "aarch64") => Some("macosx_11_0_arm64"),
        _ => None,
    }
}

/// Take the uv program out of its wheel (a zip file).
fn extract_uv(wheel: &Path, target: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(File::open(wheel)?)
        .map_err(|_| HubError("The Python installer download is damaged.".into()))?;
    let wanted = format!("/scripts/uv{EXE}");
    let index = (0..archive.len())
        .find(|&i| archive.by_index(i).ok().and_then(|f| f.name().ok().map(|n| n.ends_with(&wanted))).unwrap_or(false))
        .ok_or_else(|| HubError("The Python installer download does not contain uv.".into()))?;
    let mut file = archive.by_index(index).map_err(|e| HubError(format!("The Python installer download is damaged ({e}).")))?;
    if let Some(folder) = target.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let temporary = target.with_extension("tmp");
    let mut out = File::create(&temporary)?;
    std::io::copy(&mut file, &mut out)?;
    drop(out);
    make_executable(&temporary);
    std::fs::rename(&temporary, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names() {
        assert_eq!(package_name("pygame==2.6.1"), "pygame");
        assert_eq!(package_name("pygame-ce>=2.4"), "pygame-ce");
        assert_eq!(package_name("numpy"), "numpy");
    }
}

//! Starting installed games and keeping an eye on them.
//!
//! Each run's output goes to `<data folder>/logs/<game>-<version>.log`. When a game exits, an
//! "exited" event is recorded; if it crashed, the event carries the end of the log so the hub can
//! say what went wrong (for example a missing Python package).

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::catalog::{Launch, LaunchKind};
use crate::installer::make_executable;
use crate::paths::{is_within, safe_name};
use crate::runtime::{env_python, ENV_DIR};
use crate::settings::Settings;
use crate::{fail, HubError, Result};

const LOG_TAIL_LINES: usize = 12;

/// Opens a file or folder the way double-clicking it would.
pub type Opener = Arc<dyn Fn(&Path) -> Result<()> + Send + Sync>;

struct Running {
    child: Mutex<Child>,
    pid: u32,
    started: f64,
    started_at: Instant,
    title: String,
    stopping: AtomicBool,
}

pub struct Launcher {
    settings: Arc<Settings>,
    logs_dir: PathBuf,
    opener: Opener,
    running: Mutex<HashMap<(String, String), Arc<Running>>>,
    events: Mutex<(VecDeque<Value>, u64)>,
}

impl Launcher {
    pub fn new(settings: Arc<Settings>, logs_dir: PathBuf, opener: Opener) -> Arc<Launcher> {
        Arc::new(Launcher {
            settings,
            logs_dir,
            opener,
            running: Mutex::new(HashMap::new()),
            events: Mutex::new((VecDeque::new(), 1)),
        })
    }

    pub fn is_running(&self, game_id: &str, version: &str) -> bool {
        self.running.lock().unwrap().contains_key(&(game_id.to_string(), version.to_string()))
    }

    pub fn running(&self) -> Vec<Value> {
        let running = self.running.lock().unwrap();
        let mut list: Vec<Value> = running
            .iter()
            .map(|((game_id, version), info)| {
                json!({"game_id": game_id, "version": version, "pid": info.pid, "started": info.started})
            })
            .collect();
        list.sort_by(|a, b| a["started"].as_f64().partial_cmp(&b["started"].as_f64()).unwrap());
        list
    }

    pub fn events_since(&self, after_id: u64) -> Vec<Value> {
        let events = self.events.lock().unwrap();
        events.0.iter().filter(|event| event["id"].as_u64().unwrap_or(0) > after_id).cloned().collect()
    }

    pub fn log_path(&self, game_id: &str, version: &str) -> PathBuf {
        self.logs_dir.join(format!("{game_id}-{}.log", safe_name(version)))
    }

    pub fn logs_dir(&self) -> &Path {
        &self.logs_dir
    }

    /// Start an installed version. `installed` is its marker info from the installer.
    pub fn launch(self: &Arc<Self>, game_id: &str, title: &str, version: &str, installed: &Value) -> Result<Value> {
        let folder = PathBuf::from(installed["path"].as_str().unwrap_or_default());
        let launch: Launch = serde_json::from_value(installed["launch"].clone())
            .map_err(|_| HubError(format!("{title} {version} has no launch details. Try reinstalling it.")))?;
        let target = folder.join(&launch.path);
        if !is_within(&target, &folder) {
            return fail("This version's launch path points outside its folder.");
        }
        if !target.is_file() {
            return fail(format!("'{}' is missing from the installed files. Try reinstalling {title} {version}.", launch.path));
        }
        if launch.kind == LaunchKind::Open {
            (self.opener)(&target)?;
            return Ok(json!({"tracked": false}));
        }
        let mut command_line = match launch.kind {
            LaunchKind::Python => {
                // The game's own Python (see runtime.rs), unless the player chose one in Settings.
                let own = env_python(&folder.join(ENV_DIR));
                if self.settings.get("python").is_empty() && own.is_file() {
                    vec![own.to_string_lossy().into_owned()]
                } else {
                    self.settings.python_command()
                }
            }
            _ => {
                make_executable(&target);
                Vec::new()
            }
        };
        command_line.push(target.to_string_lossy().into_owned());
        command_line.extend(launch.args.iter().cloned());

        let key = (game_id.to_string(), version.to_string());
        let mut running = self.running.lock().unwrap();
        if running.contains_key(&key) {
            return fail(format!("{title} {version} is already running."));
        }
        std::fs::create_dir_all(&self.logs_dir)?;
        let log_path = self.log_path(game_id, version);
        let mut log = File::create(&log_path)?;
        let workdir = target.parent().unwrap_or(&folder).to_path_buf();
        writeln!(log, "# {title} {version}, started {}", crate::installer::utc_now())?;
        writeln!(log, "# {}\n# in {}\n", quote_command(&command_line), workdir.display())?;
        let mut command = Command::new(&command_line[0]);
        command
            .args(&command_line[1..])
            .current_dir(&workdir)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        if launch.kind == LaunchKind::Python {
            command.env("PYTHONUNBUFFERED", "1").env("PYTHONIOENCODING", "utf-8");
        }
        detach(&mut command, launch.kind);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return fail(if launch.kind == LaunchKind::Python {
                    format!("Python was not found ('{}'). Set the Python interpreter in Settings.", command_line[0])
                } else {
                    format!("Could not find {} to start it.", target.file_name().unwrap_or_default().to_string_lossy())
                });
            }
            Err(error) => return fail(format!("Could not start {title}: {error}")),
        };
        let pid = child.id();
        let info = Arc::new(Running {
            child: Mutex::new(child),
            pid,
            started: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
            started_at: Instant::now(),
            title: title.to_string(),
            stopping: AtomicBool::new(false),
        });
        running.insert(key.clone(), info.clone());
        drop(running);
        let launcher = Arc::clone(self);
        std::thread::Builder::new()
            .name(format!("watch-{game_id}"))
            .spawn(move || launcher.watch(key, info, log_path))
            .map_err(|e| HubError(format!("Could not watch the game: {e}")))?;
        Ok(json!({"tracked": true, "pid": pid}))
    }

    pub fn stop(&self, game_id: &str, version: &str) -> Result<()> {
        let info = self.running.lock().unwrap().get(&(game_id.to_string(), version.to_string())).cloned();
        let Some(info) = info else { return fail("That game is not running.") };
        info.stopping.store(true, Ordering::Relaxed);
        let _ = info.child.lock().unwrap().kill();
        Ok(())
    }

    fn watch(&self, key: (String, String), info: Arc<Running>, log_path: PathBuf) {
        // Poll rather than wait(), so stop() can lock the child and kill it meanwhile.
        let status = loop {
            match info.child.lock().unwrap().try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(_) => break None,
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        let code = status.and_then(|s| s.code());
        let stopped = info.stopping.load(Ordering::Relaxed);
        let crashed = !stopped && code != Some(0);
        let mut events = self.events.lock().unwrap();
        let id = events.1;
        events.1 += 1;
        events.0.push_back(json!({
            "id": id, "type": "exited", "game_id": key.0, "version": key.1, "title": info.title,
            "code": code, "stopped": stopped,
            "seconds": (info.started_at.elapsed().as_secs_f64() * 10.0).round() / 10.0,
            "log_tail": if crashed { read_tail(&log_path, LOG_TAIL_LINES) } else { String::new() },
        }));
        while events.0.len() > 50 {
            events.0.pop_front();
        }
        drop(events);
        self.running.lock().unwrap().remove(&key);
    }
}

/// The end of a log, without the hub's own `# ...` header lines.
pub fn read_tail(path: &Path, lines: usize) -> String {
    let Ok(mut file) = File::open(path) else { return String::new() };
    let length = file.seek(SeekFrom::End(0)).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(length.saturating_sub(8192)));
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    let text = String::from_utf8_lossy(&bytes);
    let body: Vec<&str> = text.lines().filter(|line| !line.starts_with("# ")).collect();
    body[body.len().saturating_sub(lines)..].join("\n").trim().to_string()
}

fn quote_command(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| if part.contains(' ') { format!("\"{part}\"") } else { part.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Games keep running if the hub closes, and Python games do not open a console window.
fn detach(command: &mut Command, kind: LaunchKind) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut flags = CREATE_NEW_PROCESS_GROUP;
        if kind == LaunchKind::Python {
            flags |= CREATE_NO_WINDOW;
        }
        command.creation_flags(flags);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _ = kind;
        command.process_group(0);
    }
}

/// The platform's own "open" command, for when no other opener is given.
pub fn system_opener() -> Opener {
    Arc::new(|path: &Path| {
        let result = if cfg!(windows) {
            Command::new("explorer").arg(path).spawn()
        } else if cfg!(target_os = "macos") {
            Command::new("open").arg(path).spawn()
        } else {
            Command::new("xdg-open").arg(path).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        };
        result.map(|_| ()).map_err(|e| HubError(format!("Could not open {}: {e}", path.display())))
    })
}

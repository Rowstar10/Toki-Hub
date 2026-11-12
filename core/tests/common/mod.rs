#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use toki_hub_core::{Hub, HubOptions};

pub const GAME_FILES: &[(&str, &str)] = &[("main.py", "print('hello from the game')\n"), ("Resources/data.txt", "data")];

/// A token in the real environment would leak into requests, so hide it from every test.
pub fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::remove_var("GITHUB_TOKEN");
        std::env::remove_var("GH_TOKEN");
    });
}

/// A fresh temporary folder, deleted afterwards.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("toki-hub-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn zip_bytes(files: &[(&str, &str)], root: Option<&str>) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in files {
        let name = match root {
            Some(root) => format!("{root}/{name}"),
            None => name.to_string(),
        };
        writer.start_file(name, options).unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

pub fn write_game(catalog: &Path, id: &str, data: &Value) -> PathBuf {
    let folder = catalog.join(id);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("game.json"), data.to_string()).unwrap();
    folder
}

pub fn game_with(versions: Vec<Value>) -> Value {
    json!({"title": "Test Game", "versions": versions})
}

pub fn version_with(version: &str, source: Value) -> Value {
    json!({"version": version, "builds": [{"platform": "any", "source": source,
                                           "launch": {"type": "python", "path": "main.py"}}]})
}

pub fn version_launching(version: &str, source: Value, launch: Value) -> Value {
    json!({"version": version, "builds": [{"platform": "any", "source": source, "launch": launch}]})
}

pub enum Reply {
    Body(u16, Vec<(String, String)>, Vec<u8>),
    /// `size` bytes, sent slowly.
    Slow(usize),
}

type Route = Box<dyn Fn() -> Reply + Send + Sync>;

/// A tiny web server for tests that records every request's headers.
pub struct FakeWeb {
    pub base: String,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    requests: Arc<Mutex<Vec<(String, HashMap<String, String>)>>>,
    server: Arc<tiny_http::Server>,
}

struct SlowReader {
    left: usize,
}

impl Read for SlowReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Ok(0);
        }
        std::thread::sleep(Duration::from_millis(20));
        let count = buf.len().min(self.left).min(64 * 1024);
        buf[..count].fill(b'x');
        self.left -= count;
        Ok(count)
    }
}

impl FakeWeb {
    pub fn new() -> FakeWeb {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let routes: Arc<Mutex<HashMap<String, Route>>> = Arc::default();
        let requests: Arc<Mutex<Vec<(String, HashMap<String, String>)>>> = Arc::default();
        let (srv, r, q) = (server.clone(), routes.clone(), requests.clone());
        std::thread::spawn(move || {
            for request in srv.incoming_requests() {
                let headers = request
                    .headers()
                    .iter()
                    .map(|h| (h.field.as_str().as_str().to_lowercase(), h.value.as_str().to_string()))
                    .collect();
                let path = request.url().to_string();
                q.lock().unwrap().push((path.clone(), headers));
                let reply = r.lock().unwrap().get(&path).map(|route| route());
                std::thread::spawn(move || {
                    let _ = match reply {
                        None => request.respond(tiny_http::Response::from_string("not found").with_status_code(404)),
                        Some(Reply::Body(status, headers, body)) => {
                            let mut response = tiny_http::Response::from_data(body).with_status_code(status);
                            for (key, value) in headers {
                                response.add_header(tiny_http::Header::from_bytes(key, value).unwrap());
                            }
                            request.respond(response)
                        }
                        Some(Reply::Slow(size)) => request.respond(tiny_http::Response::new(
                            200.into(),
                            vec![],
                            SlowReader { left: size },
                            Some(size),
                            None,
                        )),
                    };
                });
            }
        });
        FakeWeb { base, routes, requests, server }
    }

    pub fn route(&self, path: &str, route: impl Fn() -> Reply + Send + Sync + 'static) {
        self.routes.lock().unwrap().insert(path.to_string(), Box::new(route));
    }

    pub fn file(&self, path: &str, data: Vec<u8>) {
        self.route(path, move || Reply::Body(200, vec![], data.clone()));
    }

    pub fn json(&self, path: &str, payload: Value) {
        let body = payload.to_string().into_bytes();
        self.route(path, move || {
            Reply::Body(200, vec![("Content-Type".into(), "application/json".into())], body.clone())
        });
    }

    pub fn redirect(&self, path: &str, to: &str) {
        let location = format!("{}{to}", self.base);
        self.route(path, move || Reply::Body(302, vec![("Location".into(), location.clone())], vec![]));
    }

    pub fn headers_for(&self, path: &str) -> Vec<HashMap<String, String>> {
        self.requests.lock().unwrap().iter().filter(|(p, _)| p == path).map(|(_, h)| h.clone()).collect()
    }
}

impl Drop for FakeWeb {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

/// A catalog folder, a data folder and a fake web server.
pub struct World {
    pub dir: TempDir,
    pub web: FakeWeb,
}

impl World {
    pub fn new() -> World {
        setup();
        let world = World { dir: TempDir::new(), web: FakeWeb::new() };
        std::fs::create_dir_all(world.catalog()).unwrap();
        world
    }

    pub fn catalog(&self) -> PathBuf {
        self.dir.path().join("catalog")
    }

    pub fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    pub fn hub(&self) -> Hub {
        let mut options = HubOptions::new(vec![self.catalog()], self.data());
        options.github_api = self.web.base.clone();
        options.github_web = self.web.base.clone();
        options.platform = "windows".into();
        let hub = Hub::new(options);
        if let Some(python) = python() {
            hub.settings.update(&[("python", python)]).unwrap();
        }
        hub
    }
}

pub fn python() -> Option<String> {
    ["python3", "python"].iter().find_map(|name| toki_hub_core::paths::which(name)).map(|p| p.to_string_lossy().into_owned())
}

pub fn wait_for<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(value) = check() {
            return value;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

pub fn install_and_wait(hub: &Hub, game: &str, version: &str) -> Value {
    let task = hub.install(game, version).unwrap();
    let id = task["id"].as_str().unwrap().to_string();
    wait_for("the install", || {
        hub.installer.tasks().into_iter().find(|t| {
            t["id"] == id.as_str() && !matches!(t["status"].as_str(), Some("queued" | "downloading" | "installing"))
        })
    })
}

pub fn listing(folder: &Path) -> Vec<String> {
    std::fs::read_dir(folder)
        .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

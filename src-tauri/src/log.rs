//! A small start-up log at `<data folder>/logs/hub.log`, since the exe has no console.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

static LOG: OnceLock<Mutex<File>> = OnceLock::new();

pub fn start(path: &Path) {
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    if let Ok(file) = File::create(path) {
        let _ = LOG.set(Mutex::new(file));
    }
}

pub fn line(message: &str) {
    if let Some(file) = LOG.get() {
        let mut file = file.lock().unwrap();
        let _ = writeln!(file, "{} {message}", toki_hub_core::installer::utc_now());
        let _ = file.flush();
    }
}

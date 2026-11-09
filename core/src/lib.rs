//! Toki Hub's logic: the catalog, downloads, installs and starting games.
//!
//! There is no UI in here; the Tauri app in `../src-tauri` shows it.

pub mod catalog;
pub mod download;
pub mod hub;
pub mod installer;
pub mod launcher;
pub mod paths;
pub mod runtime;
pub mod settings;
pub mod sources;
pub mod updates;
pub mod versions;

pub use hub::{Hub, HubOptions};

/// A problem worth showing to the player as-is (a bad catalog entry, a failed download...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubError(pub String);

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HubError {}

impl From<std::io::Error> for HubError {
    fn from(error: std::io::Error) -> Self {
        HubError(format!("File error: {error}"))
    }
}

pub type Result<T> = std::result::Result<T, HubError>;

/// `Err(HubError(message))`, for brevity.
pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(HubError(message.into()))
}

//! The phone's own services, for Toki Hub on Android, where games are apps: Android installs,
//! starts and removes them, and Toki Hub asks it to. The app provides these (src-tauri, through
//! its Android plugin); on a computer there is no device and games are installed as files.

use std::path::Path;

use crate::Result;

pub trait Device: Send + Sync {
    /// The installed app's version name, or None if it is not installed.
    fn installed(&self, package: &str) -> Option<String>;
    /// Hand an APK to Android's installer, which asks the player to confirm.
    fn install(&self, apk: &Path) -> Result<()>;
    /// Start the app.
    fn launch(&self, package: &str) -> Result<()>;
    /// Ask Android to remove the app (it asks the player to confirm).
    fn uninstall(&self, package: &str) -> Result<()>;
}

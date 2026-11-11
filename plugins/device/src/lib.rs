//! On Android, Toki Hub's games are apps. This plugin's Kotlin half (`android/`) asks Android to
//! install an APK, start an app, remove one, and say which version of an app is installed.
//! On a computer it does nothing.

#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

#[cfg(target_os = "android")]
pub use phone::Phone;

#[derive(Serialize)]
struct Name<'a> {
    name: &'a str,
}

#[derive(Serialize)]
struct File<'a> {
    path: &'a str,
}

#[derive(Deserialize)]
struct Installed {
    version: Option<String>,
}

#[derive(Deserialize)]
pub struct Folders {
    /// The app's own files (where downloads must go for Android's installer to read them).
    pub files: String,
    pub cache: String,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("tokidevice")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                let handle = _api.register_android_plugin("com.rowstar10.tokihub.device", "DevicePlugin")?;
                _app.manage(Phone(handle));
            }
            Ok(())
        })
        .build()
}

/// The phone, once the plugin is set up (Android only).
pub fn phone<R: Runtime, M: Manager<R>>(manager: &M) -> Option<tauri::State<'_, Phone<R>>> {
    manager.try_state::<Phone<R>>()
}

#[cfg(not(target_os = "android"))]
pub struct Phone<R: Runtime>(std::marker::PhantomData<fn() -> R>);

#[cfg(target_os = "android")]
mod phone {
    use super::*;
    use tauri::plugin::PluginHandle;

    /// Calls into the Kotlin plugin. Each call waits for Android's answer, so never make one on
    /// the main thread.
    pub struct Phone<R: Runtime>(pub PluginHandle<R>);

    impl<R: Runtime> Phone<R> {
        fn call<T: serde::de::DeserializeOwned>(&self, command: &str, payload: impl Serialize) -> Result<T, String> {
            self.0.run_mobile_plugin(command, payload).map_err(|error| match error {
                tauri::plugin::mobile::PluginInvokeError::InvokeRejected(rejected) => {
                    rejected.message.unwrap_or_else(|| format!("Android said no to {command}"))
                }
                other => other.to_string(),
            })
        }

        pub fn folders(&self) -> Result<Folders, String> {
            self.call("folders", ())
        }

        pub fn installed(&self, package: &str) -> Option<String> {
            self.call::<Installed>("installed", Name { name: package }).ok().and_then(|found| found.version)
        }

        pub fn install(&self, apk: &str) -> Result<(), String> {
            self.call::<serde_json::Value>("install", File { path: apk }).map(|_| ())
        }

        pub fn launch(&self, package: &str) -> Result<(), String> {
            self.call::<serde_json::Value>("launch", Name { name: package }).map(|_| ())
        }

        pub fn uninstall(&self, package: &str) -> Result<(), String> {
            self.call::<serde_json::Value>("uninstall", Name { name: package }).map(|_| ())
        }
    }
}

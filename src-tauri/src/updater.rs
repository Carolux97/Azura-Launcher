//! Launcher self-update, on top of Tauri's updater plugin.
//!
//! The plugin reads `latest.json` from the endpoint in `tauri.conf.json` (a release asset of the
//! releases repository), compares its version with the running one and, when newer, downloads the
//! update archive and checks its signature against the public key in the config before replacing
//! the app. `scripts/release-launcher.sh` and `scripts/publish-launcher.sh` produce both files.
//!
//! The game runtime is not downloaded: it is built on the player's machine from their own copy of
//! the game. A new launcher brings a new player kit, and the builder then rebuilds or relinks the
//! runtime from it (`builder.rs`).

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// Event carrying the download progress of an update.
pub const PROGRESS_EVENT: &str = "update-progress";

/// A newer launcher version, sent to the interface as JSON.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
}

/// Bytes downloaded so far and the total, when the server reports it.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// Asks the update endpoint for a newer version. `Ok(None)` means the launcher is up to date. An
/// unreachable endpoint (offline, or a repository that is still private) is an `Err`, which the
/// interface treats as "no update" without bothering the user.
pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    // The development launcher never updates itself: an update would turn it into the published one.
    if cfg!(feature = "dev") {
        return Ok(None);
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    Ok(update.map(|update| UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: update.body.clone(),
    }))
}

/// Downloads and installs the newer version, reporting progress, then restarts the launcher into
/// it. Only returns when something failed.
pub async fn install(app: AppHandle) -> Result<(), String> {
    if cfg!(feature = "dev") {
        return Err("the development launcher does not update itself".to_owned());
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no update available".to_owned())?;

    let mut downloaded: u64 = 0;
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                let _ = app.emit(PROGRESS_EVENT, Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;

    app.restart();
}

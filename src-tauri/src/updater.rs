//! Automatic updates from GitHub releases.
//!
//! Release builds check `latest.json` on the latest published release shortly
//! after launch and every few hours. A newer, correctly signed version is
//! downloaded in the background; the footer and tray then offer "Restart to
//! update". Installing is always the user's choice, because restarting stops
//! any dev servers Echo is running.

use crate::AppState;
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadyUpdate {
    pub version: String,
    pub notes: Option<String>,
}

/// A downloaded update waiting for the user to restart.
#[derive(Default)]
pub struct UpdateState {
    ready: Mutex<Option<(Update, Vec<u8>)>>,
    checking: Mutex<bool>,
}

impl UpdateState {
    pub fn ready(&self) -> Option<ReadyUpdate> {
        let ready = self.ready.lock().unwrap_or_else(|e| e.into_inner());
        ready.as_ref().map(|(u, _)| ReadyUpdate { version: u.version.clone(), notes: u.body.clone() })
    }
}

/// Updates only make sense for installed release builds.
pub fn enabled() -> bool {
    !cfg!(debug_assertions)
}

/// Check now, then on a timer.
pub fn start(app: &AppHandle) {
    if !enabled() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            check(&app, false).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Look for a newer release and download it. `user` = asked from the tray, so
/// say what happened even when there is nothing new.
pub async fn check(app: &AppHandle, user: bool) {
    let state = app.state::<UpdateState>();
    if let Some(ready) = state.ready() {
        if user {
            notify(app, &format!("Echo {} is ready", ready.version), "Choose Restart to update in the tray menu.");
        }
        return;
    }
    {
        let mut checking = state.checking.lock().unwrap_or_else(|e| e.into_inner());
        if *checking {
            return;
        }
        *checking = true;
    }
    let result = find_and_download(app).await;
    *state.checking.lock().unwrap_or_else(|e| e.into_inner()) = false;
    match result {
        Ok(Some((update, bytes))) => {
            let ready = ReadyUpdate { version: update.version.clone(), notes: update.body.clone() };
            log::info!("update {} downloaded", ready.version);
            *state.ready.lock().unwrap_or_else(|e| e.into_inner()) = Some((update, bytes));
            let _ = app.emit("update-ready", &ready);
            crate::tray::refresh_now(app);
            if user {
                notify(app, &format!("Echo {} is ready", ready.version), "Choose Restart to update in the tray menu.");
            }
        }
        Ok(None) => {
            log::info!("no update available");
            if user {
                notify(
                    app,
                    "Echo is up to date",
                    &format!("You have the latest version, {}.", app.package_info().version),
                );
            }
        }
        Err(e) => {
            log::warn!("update check failed: {e}");
            if user {
                notify(app, "Couldn't check for updates", &e);
            }
        }
    }
}

async fn find_and_download(app: &AppHandle) -> Result<Option<(Update, Vec<u8>)>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else { return Ok(None) };
    log::info!("downloading update {}", update.version);
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    Ok(Some((update, bytes)))
}

/// Stop dev servers and watchers, install the downloaded update and relaunch.
pub fn install_and_restart(app: &AppHandle) -> Result<(), String> {
    let taken = app.state::<UpdateState>().ready.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some((update, bytes)) = taken else { return Err("No update is ready.".into()) };
    log::info!("installing update {}", update.version);
    // Stop dev servers and flush state first: on Windows the installer closes
    // the app as part of installing.
    if let Some(state) = app.try_state::<AppState>() {
        state.core.shutdown();
    }
    if let Err(e) = update.install(bytes) {
        // Echo is already shut down, so relaunch the current version.
        log::error!("update install failed: {e}");
        notify(app, "Couldn't install the update", &e.to_string());
    }
    app.restart();
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("notification failed: {e}");
    }
}

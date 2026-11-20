// Echo: a tray app that keeps one remote branch synced into a local checkout.
// All git work lives in `echo-core`; this crate wires it to Tauri (windows,
// tray, notifications, autostart, IPC).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod platform;
mod tray;

use echo_core::{Core, CoreEvent, EventSink};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_notification::NotificationExt;

pub const HIDDEN_ARG: &str = "--hidden";

pub struct AppState {
    pub core: Core,
}

/// Forwards core events to the webview, the tray and the OS notification centre.
struct TauriSink {
    app: AppHandle,
    tray: tray::TrayRefresher,
}

impl EventSink for TauriSink {
    fn emit(&self, event: CoreEvent) {
        let app = &self.app;
        let r = match &event {
            CoreEvent::ReposChanged => app.emit("repos-changed", ()),
            CoreEvent::BranchesUpdated { repo_id, snapshot } => {
                app.emit("branches-updated", serde_json::json!({ "repoId": repo_id, "snapshot": snapshot }))
            }
            CoreEvent::SyncStatusChanged { repo_id, status } => {
                app.emit("sync-status-changed", serde_json::json!({ "repoId": repo_id, "status": status }))
            }
            CoreEvent::SyncEvent { repo_id, event } => {
                app.emit("sync-event", serde_json::json!({ "repoId": repo_id, "event": event }))
            }
            CoreEvent::FlagsChanged { repo_id, flags } => {
                app.emit("flags-changed", serde_json::json!({ "repoId": repo_id, "flags": flags }))
            }
            CoreEvent::SettingsChanged => app.emit("settings-changed", ()),
            // Notifications are informational only: they never change the
            // selected repo or branch.
            CoreEvent::Notify { title, body, .. } => {
                if let Err(e) = app.notification().builder().title(title).body(body).show() {
                    log::warn!("notification failed: {e}");
                }
                Ok(())
            }
        };
        if let Err(e) = r {
            log::warn!("emit failed: {e}");
        }
        if !matches!(event, CoreEvent::Notify { .. } | CoreEvent::FlagsChanged { .. }) {
            self.tray.request();
        }
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Make the OS login item match the setting. Skipped in debug builds so
/// `tauri dev` doesn't register the dev binary.
pub fn apply_autostart(app: &AppHandle, enabled: bool) {
    if cfg!(debug_assertions) {
        return;
    }
    let al = app.autolaunch();
    let current = al.is_enabled().unwrap_or(!enabled);
    let r = match (enabled, current) {
        (true, false) => al.enable(),
        (false, true) => al.disable(),
        _ => Ok(()),
    };
    if let Err(e) = r {
        log::warn!("could not update launch at login: {e}");
    }
}

fn main() {
    let app = tauri::Builder::default()
        // Must be first: a second launch focuses the existing window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .level(log::LevelFilter::Info)
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout))
                .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                    file_name: Some("echo".into()),
                }))
                .max_file_size(2_000_000)
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![HIDDEN_ARG])))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all() - tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = app.path().app_data_dir()?;
            let sink = Arc::new(TauriSink { app: handle.clone(), tray: tray::TrayRefresher::new(handle.clone()) });
            let version = app.package_info().version.to_string();
            let core = tauri::async_runtime::block_on(Core::start(data_dir, sink, &version));
            let settings = core.get_settings();
            app.manage(AppState { core });

            tray::create(&handle)?;
            apply_autostart(&handle, settings.launch_at_login);

            let launched_hidden = std::env::args().any(|a| a == HIDDEN_ARG);
            if !(launched_hidden && settings.start_hidden) {
                show_main(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Closing hides to the tray; quitting is from the tray or Cmd+Q.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Focused(true) => {
                let app = window.app_handle();
                // Focus only speeds up polling; it never changes the selection.
                if let Some(state) = app.try_state::<AppState>() {
                    state.core.on_focus();
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_repos,
            commands::add_repo,
            commands::remove_repo,
            commands::locate_repo,
            commands::update_repo_config,
            commands::get_branches,
            commands::mark_seen,
            commands::get_branch_log,
            commands::start_sync,
            commands::stop_sync,
            commands::pause_sync,
            commands::resume_sync,
            commands::sync_now,
            commands::discard_changes,
            commands::move_untracked_aside,
            commands::reset_to_remote,
            commands::dismiss_flag,
            commands::get_settings,
            commands::update_settings,
            commands::default_flag_patterns,
            commands::open_in_file_manager,
            commands::open_in_terminal,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Echo");

    app.run(|app, event| match event {
        // macOS: clicking the dock icon re-opens the hidden window.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main(app),
        RunEvent::Exit => {
            if let Some(state) = app.try_state::<AppState>() {
                state.core.shutdown();
            }
        }
        _ => {}
    });
}

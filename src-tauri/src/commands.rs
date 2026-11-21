//! Thin `#[tauri::command]` wrappers over [`echo_core::Core`].

use crate::AppState;
use echo_core::devserver::{is_local_url, DevServerSnapshot, DevServerStatus};
use echo_core::flags::{default_patterns, FlagKind};
use echo_core::github::PullRequestSnapshot;
use echo_core::store::{ActiveFlag, Config};
use echo_core::types::*;
use echo_core::{CmdError, CmdErrorKind};
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, State};

type R<T> = Result<T, CmdError>;

fn invalid(msg: impl Into<String>) -> CmdError {
    CmdError { kind: CmdErrorKind::Invalid, message: msg.into(), paths: vec![] }
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    state.core.app_info()
}

#[tauri::command]
pub fn list_repos(state: State<'_, AppState>) -> Vec<RepoSummary> {
    state.core.list_repos()
}

#[tauri::command]
pub async fn add_repo(state: State<'_, AppState>, path: String) -> R<RepoSummary> {
    state.core.add_repo(&path).await
}

#[tauri::command]
pub fn remove_repo(state: State<'_, AppState>, id: String) -> R<()> {
    state.core.remove_repo(&id)
}

#[tauri::command]
pub async fn locate_repo(state: State<'_, AppState>, id: String, path: String) -> R<RepoSummary> {
    state.core.locate_repo(&id, &path).await
}

#[tauri::command]
pub fn update_repo_config(state: State<'_, AppState>, id: String, patch: Value) -> R<RepoSummary> {
    state.core.update_repo_config(&id, patch)
}

#[tauri::command]
pub fn get_branches(state: State<'_, AppState>, repo_id: String) -> R<BranchSnapshot> {
    state.core.get_branches(&repo_id)
}

#[tauri::command]
pub fn mark_seen(state: State<'_, AppState>, repo_id: String, branch: Option<String>) -> R<()> {
    state.core.mark_seen(&repo_id, branch.as_deref())
}

#[tauri::command]
pub async fn get_branch_log(
    state: State<'_, AppState>,
    repo_id: String,
    branch: String,
    n: usize,
) -> R<Vec<CommitSummary>> {
    state.core.get_branch_log(&repo_id, &branch, n).await
}

#[tauri::command]
pub async fn start_sync(state: State<'_, AppState>, repo_id: String, branch: String, discard: bool) -> R<SyncStatus> {
    state.core.start_sync(&repo_id, &branch, discard).await
}

#[tauri::command]
pub fn stop_sync(state: State<'_, AppState>, repo_id: String) -> R<SyncStatus> {
    state.core.stop_sync(&repo_id)
}

#[tauri::command]
pub fn pause_sync(state: State<'_, AppState>, repo_id: String) -> R<SyncStatus> {
    state.core.pause_sync(&repo_id)
}

#[tauri::command]
pub async fn resume_sync(state: State<'_, AppState>, repo_id: String) -> R<SyncStatus> {
    state.core.resume_sync(&repo_id).await
}

#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>, repo_id: String) -> R<SyncStatus> {
    state.core.sync_now(&repo_id).await
}

#[tauri::command]
pub async fn discard_changes(state: State<'_, AppState>, repo_id: String) -> R<SyncStatus> {
    state.core.discard_changes(&repo_id).await
}

#[tauri::command]
pub async fn move_untracked_aside(state: State<'_, AppState>, repo_id: String, paths: Vec<String>) -> R<Vec<String>> {
    state.core.move_untracked_aside(&repo_id, &paths).await
}

#[tauri::command]
pub async fn reset_to_remote(state: State<'_, AppState>, repo_id: String) -> R<SyncStatus> {
    state.core.reset_to_remote(&repo_id).await
}

#[tauri::command]
pub fn dismiss_flag(state: State<'_, AppState>, repo_id: String, kind: FlagKind) -> R<Vec<ActiveFlag>> {
    state.core.dismiss_flag(&repo_id, kind)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Config {
    state.core.get_settings()
}

#[tauri::command]
pub async fn update_settings(app: AppHandle, state: State<'_, AppState>, patch: Value) -> R<Config> {
    let touches_autostart = patch.get("launchAtLogin").is_some();
    let config = state.core.update_settings(patch).await?;
    if touches_autostart {
        crate::apply_autostart(&app, config.launch_at_login);
    }
    Ok(config)
}

#[tauri::command]
pub fn default_flag_patterns() -> Value {
    serde_json::json!({
        "deps": default_patterns(FlagKind::Deps),
        "migrations": default_patterns(FlagKind::Migrations),
        "env": default_patterns(FlagKind::Env),
    })
}

fn repo_path(state: &AppState, id: &str) -> R<PathBuf> {
    state
        .core
        .list_repos()
        .into_iter()
        .find(|r| r.id == id)
        .map(|r| PathBuf::from(r.config.path))
        .ok_or_else(|| invalid("repo not found"))
}

#[tauri::command]
pub fn open_in_file_manager(state: State<'_, AppState>, id: String) -> R<()> {
    crate::platform::open_in_file_manager(&repo_path(&state, &id)?).map_err(|e| invalid(e.to_string()))
}

#[tauri::command]
pub fn open_in_terminal(state: State<'_, AppState>, id: String) -> R<()> {
    crate::platform::open_in_terminal(&repo_path(&state, &id)?).map_err(|e| invalid(e.to_string()))
}

#[tauri::command]
pub fn get_pull_requests(state: State<'_, AppState>, repo_id: String) -> R<PullRequestSnapshot> {
    state.core.get_pull_requests(&repo_id)
}

/// URLs are built in Rust from the repo's origin and gh's output; the webview
/// can only ask for "this repo" or "this branch's PR", never an arbitrary URL.
fn open_github(url: &str) -> R<()> {
    if !url.starts_with("https://github.com/") {
        return Err(invalid("not a GitHub link"));
    }
    crate::platform::open_url(url).map_err(|e| invalid(e.to_string()))
}

#[tauri::command]
pub fn open_on_github(state: State<'_, AppState>, repo_id: String) -> R<()> {
    open_github(&state.core.github_url(&repo_id)?)
}

/// GitHub's branches page, where branches Echo can't delete can be removed by hand.
#[tauri::command]
pub fn open_branches_on_github(state: State<'_, AppState>, repo_id: String) -> R<()> {
    open_github(&format!("{}/branches", state.core.github_url(&repo_id)?))
}

#[tauri::command]
pub async fn delete_remote_branch(state: State<'_, AppState>, repo_id: String, branch: String, sha: String) -> R<()> {
    state.core.delete_remote_branch(&repo_id, &branch, &sha).await
}

#[tauri::command]
pub fn open_pull_request(state: State<'_, AppState>, repo_id: String, branch: String) -> R<()> {
    open_github(&state.core.pull_request_url(&repo_id, &branch)?)
}

// ---- Dev server ----
// The webview sends the command text; Echo runs it from the repo root in the
// user's shell. That is the feature, and the same thing the user could type
// in their own terminal.

#[tauri::command]
pub async fn dev_server_start(
    state: State<'_, AppState>,
    repo_id: String,
    command: String,
    cols: u16,
    rows: u16,
) -> R<DevServerStatus> {
    state.core.dev_server_start(&repo_id, &command, cols, rows).await
}

#[tauri::command]
pub fn dev_server_stop(state: State<'_, AppState>, repo_id: String) -> R<DevServerStatus> {
    state.core.dev_server_stop(&repo_id)
}

#[tauri::command]
pub async fn dev_server_restart(
    state: State<'_, AppState>,
    repo_id: String,
    cols: u16,
    rows: u16,
) -> R<DevServerStatus> {
    state.core.dev_server_restart(&repo_id, cols, rows).await
}

#[tauri::command]
pub fn dev_server_write(state: State<'_, AppState>, repo_id: String, data: String) -> R<()> {
    state.core.dev_server_write(&repo_id, &data)
}

#[tauri::command]
pub fn dev_server_resize(state: State<'_, AppState>, repo_id: String, cols: u16, rows: u16) -> R<()> {
    state.core.dev_server_resize(&repo_id, cols, rows)
}

#[tauri::command]
pub fn dev_server_snapshot(state: State<'_, AppState>, repo_id: String) -> DevServerSnapshot {
    state.core.dev_server_snapshot(&repo_id)
}

/// Opens the local URL the dev server printed. The URL comes from Rust and
/// is checked to be http(s) on localhost.
#[tauri::command]
pub fn open_dev_server_url(state: State<'_, AppState>, repo_id: String) -> R<()> {
    let url = state.core.dev_server_url(&repo_id)?;
    if !is_local_url(&url) {
        return Err(invalid("not a local URL"));
    }
    crate::platform::open_url(&url).map_err(|e| invalid(e.to_string()))
}

#[tauri::command]
pub fn pin_dev_command(state: State<'_, AppState>, repo_id: String, command: String, name: String) -> R<RepoSummary> {
    state.core.pin_dev_command(&repo_id, &command, &name)
}

#[tauri::command]
pub fn unpin_dev_command(state: State<'_, AppState>, repo_id: String, command: String) -> R<RepoSummary> {
    state.core.unpin_dev_command(&repo_id, &command)
}

// ---- Updates ----

/// A downloaded update waiting for a restart, if any.
#[tauri::command]
pub fn update_status(state: State<'_, crate::updater::UpdateState>) -> Option<crate::updater::ReadyUpdate> {
    state.ready()
}

/// Install the downloaded update and relaunch. Stops running dev servers.
#[tauri::command]
pub fn install_update(app: AppHandle) -> R<()> {
    crate::updater::install_and_restart(&app).map_err(invalid)
}

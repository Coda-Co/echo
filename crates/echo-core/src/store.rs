//! Persisted config (`config.json`) and seen-state (`state.json`).
//!
//! Both files live in the app data directory. Unknown fields are preserved on
//! round-trip (`extra`), writes are atomic (temp file + rename), and a file that
//! fails to parse is backed up rather than overwritten.

use crate::flags::FlagKind;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CONFIG_VERSION: u32 = 1;
pub const STATE_VERSION: u32 = 1;

fn default_true() -> bool {
    true
}
fn default_poll() -> u64 {
    15
}
fn default_sync() -> u64 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPrefs {
    #[serde(default = "default_true")]
    pub pulled: bool,
    #[serde(default = "default_true")]
    pub force_pushed: bool,
    #[serde(default = "default_true")]
    pub blocked: bool,
    #[serde(default)]
    pub new_branch: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self { pulled: true, force_pushed: true, blocked: true, new_branch: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_path: Option<String>,
    #[serde(default = "default_true")]
    pub launch_at_login: bool,
    #[serde(default = "default_true")]
    pub start_hidden: bool,
    #[serde(default = "default_poll")]
    pub branch_poll_seconds: u64,
    #[serde(default = "default_sync")]
    pub sync_base_seconds: u64,
    #[serde(default)]
    pub notifications: NotificationPrefs,
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            git_path: None,
            launch_at_login: true,
            start_hidden: true,
            branch_poll_seconds: 15,
            sync_base_seconds: 4,
            notifications: NotificationPrefs::default(),
            repos: Vec::new(),
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FilterMode {
    All,
    Prefix,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BranchFilter {
    pub mode: FilterMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

impl Default for BranchFilter {
    fn default() -> Self {
        Self { mode: FilterMode::Prefix, prefix: Some("claude/".into()) }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ForcePushPolicy {
    #[default]
    Reset,
    Ask,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlagPatterns {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deps: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrations: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,
}

impl FlagPatterns {
    pub fn get(&self, kind: FlagKind) -> Option<&Vec<String>> {
        match kind {
            FlagKind::Deps => self.deps.as_ref(),
            FlagKind::Migrations => self.migrations.as_ref(),
            FlagKind::Env => self.env.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoConfig {
    pub id: String,
    pub path: String,
    pub display_name: String,
    #[serde(default = "default_true")]
    pub polling_enabled: bool,
    #[serde(default)]
    pub branch_filter: BranchFilter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_branch: Option<String>,
    #[serde(default)]
    pub sync_paused: bool,
    #[serde(default)]
    pub force_push_policy: ForcePushPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrate_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag_patterns: Option<FlagPatterns>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl RepoConfig {
    pub fn new(id: String, path: String, display_name: String) -> Self {
        Self {
            id,
            path,
            display_name,
            polling_enabled: true,
            branch_filter: BranchFilter::default(),
            synced_branch: None,
            sync_paused: false,
            force_push_policy: ForcePushPolicy::Reset,
            install_command: None,
            migrate_command: None,
            flag_patterns: None,
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveFlag {
    pub kind: FlagKind,
    pub files: Vec<String>,
    /// Unix millis when last raised.
    pub at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoState {
    #[serde(default)]
    pub last_seen: BTreeMap<String, String>,
    /// Last sha the app itself put each branch at. Used to tell a force-push
    /// (local == last synced) from local commits (local != last synced).
    #[serde(default)]
    pub last_synced: BTreeMap<String, String>,
    #[serde(default)]
    pub active_flags: Vec<ActiveFlag>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

const MAX_FLAG_FILES: usize = 50;

impl RepoState {
    /// Merge newly detected flags into the active list.
    pub fn raise_flags(&mut self, detected: Vec<(FlagKind, Vec<String>)>, now_ms: i64) -> bool {
        let mut changed = false;
        for (kind, files) in detected {
            changed = true;
            match self.active_flags.iter_mut().find(|f| f.kind == kind) {
                Some(f) => {
                    for file in files {
                        if !f.files.contains(&file) {
                            f.files.push(file);
                        }
                    }
                    f.files.truncate(MAX_FLAG_FILES);
                    f.at = now_ms;
                }
                None => {
                    let mut files = files;
                    files.truncate(MAX_FLAG_FILES);
                    self.active_flags.push(ActiveFlag { kind, files, at: now_ms });
                }
            }
        }
        self.active_flags.sort_by_key(|f| f.kind);
        changed
    }

    pub fn dismiss_flag(&mut self, kind: FlagKind) -> bool {
        let before = self.active_flags.len();
        self.active_flags.retain(|f| f.kind != kind);
        before != self.active_flags.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct State {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub repos: BTreeMap<String, RepoState>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for State {
    fn default() -> Self {
        Self { version: STATE_VERSION, repos: BTreeMap::new(), extra: Map::new() }
    }
}

/// Locations of the two store files.
#[derive(Debug, Clone)]
pub struct StorePaths {
    pub dir: PathBuf,
}

impl StorePaths {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    pub fn config(&self) -> PathBuf {
        self.dir.join("config.json")
    }
    pub fn state(&self) -> PathBuf {
        self.dir.join("state.json")
    }
}

pub fn load_config(path: &Path) -> Config {
    let mut c: Config = load_json(path).unwrap_or_default();
    // Migrations: v0 (or missing) -> v1 has no structural changes.
    if c.version < CONFIG_VERSION {
        c.version = CONFIG_VERSION;
    }
    c
}

pub fn load_state(path: &Path) -> State {
    let mut s: State = load_json(path).unwrap_or_default();
    if s.version < STATE_VERSION {
        s.version = STATE_VERSION;
    }
    s
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::error!("could not read {}: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            let backup = path.with_extension(format!("json.bad-{}", now_ms()));
            log::error!("{} is invalid ({e}); backing up to {}", path.display(), backup.display());
            let _ = std::fs::rename(path, &backup);
            None
        }
    }
}

/// Atomically write pretty JSON.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_unknown_fields_roundtrip() {
        let json = r#"{"version":0,"futureThing":{"a":1},"repos":[{"id":"x","path":"/r","displayName":"r","weird":true}]}"#;
        let c: Config = serde_json::from_str(json).unwrap();
        assert!(c.launch_at_login);
        assert_eq!(c.branch_poll_seconds, 15);
        assert_eq!(c.sync_base_seconds, 4);
        assert_eq!(c.repos[0].branch_filter, BranchFilter::default());
        assert_eq!(c.repos[0].force_push_policy, ForcePushPolicy::Reset);
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["futureThing"]["a"], 1);
        assert_eq!(back["repos"][0]["weird"], true);
    }

    #[test]
    fn load_missing_and_corrupt() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.json");
        assert_eq!(load_config(&p), Config::default());
        std::fs::write(&p, "{not json").unwrap();
        assert_eq!(load_config(&p), Config::default());
        assert!(!p.exists(), "corrupt file is moved aside, not deleted");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn save_and_reload() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/state.json");
        let mut s = State::default();
        s.repos.entry("r".into()).or_default().last_seen.insert("main".into(), "abc".into());
        save_json(&p, &s).unwrap();
        assert_eq!(load_state(&p), s);
    }

    #[test]
    fn flags_merge_and_dismiss() {
        let mut r = RepoState::default();
        assert!(r.raise_flags(vec![(FlagKind::Deps, vec!["a".into()])], 1));
        r.raise_flags(vec![(FlagKind::Deps, vec!["a".into(), "b".into()]), (FlagKind::Env, vec![".env.example".into()])], 2);
        assert_eq!(r.active_flags.len(), 2);
        assert_eq!(r.active_flags[0].files, vec!["a", "b"]);
        assert_eq!(r.active_flags[0].at, 2);
        assert!(r.dismiss_flag(FlagKind::Deps));
        assert!(!r.dismiss_flag(FlagKind::Deps));
        assert_eq!(r.active_flags.len(), 1);
    }
}

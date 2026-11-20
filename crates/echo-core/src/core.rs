//! The app core: owns config/state, one watcher (poll + sync loop) per repo,
//! and the operations exposed to the UI. UI-agnostic: events go out through
//! an [`EventSink`].

use crate::branches::{self, PollCache};
use crate::flags::{detect_install_command, FlagKind, FlagMatcher};
use crate::git::{Git, GitError, GitErrorKind};
use crate::interval::AdaptiveInterval;
use crate::notify::{Note, Offer, RateLimiter};
use crate::repo;
use crate::store::{self, now_ms, ActiveFlag, BranchFilter, Config, FilterMode, RepoConfig, State, StorePaths};
use crate::sync::{self, SwitchError, SyncCtx, TickResult};
use crate::types::*;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{watch, Notify};

/// Events pushed to the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CoreEvent {
    ReposChanged,
    #[serde(rename_all = "camelCase")]
    BranchesUpdated {
        repo_id: String,
        snapshot: BranchSnapshot,
    },
    #[serde(rename_all = "camelCase")]
    SyncStatusChanged {
        repo_id: String,
        status: SyncStatus,
    },
    #[serde(rename_all = "camelCase")]
    SyncEvent {
        repo_id: String,
        event: SyncEvent,
    },
    #[serde(rename_all = "camelCase")]
    FlagsChanged {
        repo_id: String,
        flags: Vec<ActiveFlag>,
    },
    #[serde(rename_all = "camelCase")]
    Notify {
        repo_id: String,
        title: String,
        body: String,
    },
    SettingsChanged,
}

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: CoreEvent);
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CmdErrorKind {
    Dirty,
    UntrackedInTheWay,
    Git,
    NotFound,
    Invalid,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct CmdError {
    pub kind: CmdErrorKind,
    pub message: String,
    pub paths: Vec<String>,
}

impl CmdError {
    fn new(kind: CmdErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), paths: vec![] }
    }
    fn not_found(what: &str) -> Self {
        Self::new(CmdErrorKind::NotFound, format!("{what} not found"))
    }
}

impl From<GitError> for CmdError {
    fn from(e: GitError) -> Self {
        Self::new(CmdErrorKind::Git, e.message)
    }
}

pub type CmdResult<T> = Result<T, CmdError>;

pub fn filter_matches(f: &BranchFilter, name: &str) -> bool {
    match f.mode {
        FilterMode::All => true,
        FilterMode::Prefix => name.starts_with(f.prefix.as_deref().unwrap_or("")),
    }
}

/// What the tray menu needs, computed in one place.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayModel {
    pub icon: TrayIconState,
    pub synced: Vec<TraySynced>,
    pub unseen: Vec<TrayUnseen>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TrayIconState {
    Idle,
    Syncing,
    Unseen,
    Blocked,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TraySynced {
    pub repo_id: String,
    pub repo_name: String,
    pub branch: String,
    pub description: String,
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayUnseen {
    pub repo_id: String,
    pub repo_name: String,
    pub branch: String,
    pub label: String,
}

// ---------------------------------------------------------------------------

struct Runtime {
    sync: SyncStatus,
    snapshot: BranchSnapshot,
    cache: PollCache,
    available: bool,
    interval: AdaptiveInterval,
    poll_auth_paused: bool,
    poll_network_backoff: bool,
    /// Kind of the last blocked/error state we notified about (once per occurrence).
    last_block_notified: Option<String>,
}

struct RepoHandle {
    id: String,
    path: PathBuf,
    /// Serialises every mutating git command for this repo.
    lock: tokio::sync::Mutex<()>,
    rt: Mutex<Runtime>,
    wake_sync: Notify,
    wake_poll: Notify,
    cancel: watch::Sender<bool>,
}

impl RepoHandle {
    fn rt(&self) -> std::sync::MutexGuard<'_, Runtime> {
        self.rt.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn exists(&self) -> bool {
        self.path.join(".git").exists()
    }
}

struct Inner {
    paths: StorePaths,
    config: Mutex<Config>,
    state: Mutex<State>,
    git: RwLock<Result<Git, GitError>>,
    repos: Mutex<HashMap<String, Arc<RepoHandle>>>,
    sink: Arc<dyn EventSink>,
    limiter: Mutex<RateLimiter>,
    state_dirty: Notify,
    app_version: String,
}

/// Cheap to clone handle to the core.
#[derive(Clone)]
pub struct Core(Arc<Inner>);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Core {
    /// Load config/state from `data_dir`, discover git and start all watchers.
    /// Must be called from within a Tokio runtime.
    pub async fn start(data_dir: impl Into<PathBuf>, sink: Arc<dyn EventSink>, app_version: &str) -> Core {
        let paths = StorePaths::new(data_dir);
        let config = store::load_config(&paths.config());
        let state = store::load_state(&paths.state());
        let git = Git::discover(config.git_path.as_deref().map(Path::new)).await;
        match &git {
            Ok(g) => log::info!("using git {} at {}", g.version_string(), g.path().display()),
            Err(e) => log::error!("git unavailable: {}", e.message),
        }
        let core = Core(Arc::new(Inner {
            paths,
            config: Mutex::new(config),
            state: Mutex::new(state),
            git: RwLock::new(git),
            repos: Mutex::new(HashMap::new()),
            sink,
            limiter: Mutex::new(RateLimiter::default()),
            state_dirty: Notify::new(),
            app_version: app_version.to_string(),
        }));
        let repos: Vec<RepoConfig> = lock(&core.0.config).repos.clone();
        for rc in repos {
            core.spawn_watcher(&rc);
        }
        core.spawn_persister();
        core.spawn_clock_watcher();
        core
    }

    fn git(&self) -> Result<Git, GitError> {
        self.0.git.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn emit(&self, e: CoreEvent) {
        self.0.sink.emit(e);
    }

    fn handle(&self, id: &str) -> CmdResult<Arc<RepoHandle>> {
        lock(&self.0.repos).get(id).cloned().ok_or_else(|| CmdError::not_found("repo"))
    }

    fn repo_config(&self, id: &str) -> Option<RepoConfig> {
        lock(&self.0.config).repos.iter().find(|r| r.id == id).cloned()
    }

    fn with_repo_config<R>(&self, id: &str, f: impl FnOnce(&mut RepoConfig) -> R) -> Option<R> {
        let mut c = lock(&self.0.config);
        let r = c.repos.iter_mut().find(|r| r.id == id).map(f);
        if r.is_some() {
            self.save_config_locked(&c);
        }
        r
    }

    fn save_config_locked(&self, c: &Config) {
        if let Err(e) = store::save_json(&self.0.paths.config(), c) {
            log::error!("could not save config: {e}");
        }
    }

    fn with_state<R>(&self, id: &str, f: impl FnOnce(&mut store::RepoState) -> R) -> R {
        let r = f(lock(&self.0.state).repos.entry(id.to_string()).or_default());
        self.0.state_dirty.notify_one();
        r
    }

    fn read_state<R>(&self, id: &str, f: impl FnOnce(&store::RepoState) -> R) -> R {
        let s = lock(&self.0.state);
        match s.repos.get(id) {
            Some(r) => f(r),
            None => f(&store::RepoState::default()),
        }
    }

    /// Write state.json now (call on quit).
    pub fn flush(&self) {
        let s = lock(&self.0.state).clone();
        if let Err(e) = store::save_json(&self.0.paths.state(), &s) {
            log::error!("could not save state: {e}");
        }
    }

    fn spawn_persister(&self) {
        let core = self.clone();
        tokio::spawn(async move {
            loop {
                core.0.state_dirty.notified().await;
                tokio::time::sleep(Duration::from_secs(1)).await;
                core.flush();
            }
        });
    }

    /// Detect sleep/wake: the wall clock jumps ahead of the monotonic clock
    /// (which doesn't advance while the machine sleeps).
    fn spawn_clock_watcher(&self) {
        let weak = Arc::downgrade(&self.0);
        tokio::spawn(async move {
            let mut last_wall = SystemTime::now();
            let mut last_mono = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let Some(inner) = weak.upgrade() else { return };
                let wall = SystemTime::now().duration_since(last_wall).unwrap_or_default();
                let mono = last_mono.elapsed();
                last_wall = SystemTime::now();
                last_mono = Instant::now();
                if wall > mono + Duration::from_secs(5) {
                    log::info!("wake detected ({}s asleep); polling now", (wall - mono).as_secs());
                    Core(inner).on_wake();
                }
            }
        });
    }

    fn spawn_watcher(&self, rc: &RepoConfig) -> Arc<RepoHandle> {
        let h = self.make_handle(rc);
        self.start_loops(&h);
        h
    }

    fn make_handle(&self, rc: &RepoConfig) -> Arc<RepoHandle> {
        let initial = match (&rc.synced_branch, rc.sync_paused) {
            (None, _) => SyncState::Off,
            (Some(_), true) => SyncState::Paused { reason: PauseReason::User },
            (Some(_), false) => SyncState::Pulling,
        };
        let (cancel, _) = watch::channel(false);
        let path = PathBuf::from(&rc.path);
        let h = Arc::new(RepoHandle {
            id: rc.id.clone(),
            path: path.clone(),
            lock: tokio::sync::Mutex::new(()),
            rt: Mutex::new(Runtime {
                sync: SyncStatus { branch: rc.synced_branch.clone(), state: initial, last_checked: None, head: None },
                snapshot: BranchSnapshot { branches: vec![], current_branch: None, polled_at: None, error: None },
                cache: PollCache::default(),
                available: path.join(".git").exists(),
                interval: AdaptiveInterval::new(Instant::now()),
                poll_auth_paused: false,
                poll_network_backoff: false,
                last_block_notified: None,
            }),
            wake_sync: Notify::new(),
            wake_poll: Notify::new(),
            cancel,
        });
        lock(&self.0.repos).insert(rc.id.clone(), h.clone());
        h
    }

    fn start_loops(&self, h: &Arc<RepoHandle>) {
        let (core, h2) = (self.clone(), h.clone());
        let mut cancel_rx = h.cancel.subscribe();
        tokio::spawn(async move {
            loop {
                core.poll_once(&h2, false).await;
                let secs = {
                    let backoff = h2.rt().poll_network_backoff;
                    if backoff {
                        60
                    } else {
                        lock(&core.0.config).branch_poll_seconds.max(5)
                    }
                };
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
                    _ = h2.wake_poll.notified() => {}
                    _ = cancel_rx.changed() => return,
                }
            }
        });

        let (core, h3) = (self.clone(), h.clone());
        let mut cancel_rx = h.cancel.subscribe();
        tokio::spawn(async move {
            loop {
                core.sync_once(&h3, false).await;
                let wait = {
                    let base = Duration::from_secs(lock(&core.0.config).sync_base_seconds.max(1));
                    h3.rt().interval.current(base, Instant::now())
                };
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = h3.wake_sync.notified() => {}
                    _ = cancel_rx.changed() => return,
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // Loops
    // -----------------------------------------------------------------------

    /// Returns false (and marks the repo unavailable) when the folder is gone.
    fn check_available(&self, h: &RepoHandle) -> bool {
        let ok = h.exists();
        let changed = {
            let mut rt = h.rt();
            let changed = rt.available != ok;
            rt.available = ok;
            changed
        };
        if changed {
            self.emit(CoreEvent::ReposChanged);
        }
        ok
    }

    async fn poll_once(&self, h: &Arc<RepoHandle>, seed_seen: bool) {
        let Some(rc) = self.repo_config(&h.id) else { return };
        if !rc.polling_enabled && !seed_seen {
            return;
        }
        if h.rt().poll_auth_paused {
            return;
        }
        let Ok(git) = self.git() else { return };
        let _g = h.lock.lock().await;
        if !self.check_available(h) {
            return;
        }
        let last_seen = self.read_state(&h.id, |s| s.last_seen.clone());
        let had_unseen = self.has_unseen(h, &rc.branch_filter);
        let mut cache = std::mem::take(&mut h.rt().cache);
        let res = branches::poll(&git, &h.path, &mut cache, &last_seen, rc.synced_branch.as_deref()).await;

        match res {
            Ok(mut out) => {
                if seed_seen && last_seen.is_empty() {
                    // First add: treat everything already on the remote as seen.
                    let tips: Vec<(String, String)> =
                        out.branches.iter().filter(|b| !b.deleted).map(|b| (b.name.clone(), b.sha.clone())).collect();
                    let seen = self.with_state(&h.id, |s| {
                        s.last_seen.extend(tips);
                        s.last_seen.clone()
                    });
                    out.branches = cache
                        .restatus(&git, &h.path, &seen, out.current_branch.as_deref(), rc.synced_branch.as_deref())
                        .await;
                }
                if !out.deleted_branches.is_empty() {
                    self.with_state(&h.id, |s| {
                        for b in &out.deleted_branches {
                            s.last_seen.remove(b);
                        }
                    });
                }
                let notify_new = lock(&self.0.config).notifications.new_branch;
                if notify_new {
                    for b in out.new_branches.iter().filter(|b| filter_matches(&rc.branch_filter, b)) {
                        self.notify(&h.id, &rc.display_name, format!("New branch: {b}"));
                    }
                }
                let snapshot = {
                    let mut rt = h.rt();
                    rt.cache = cache;
                    rt.poll_network_backoff = false;
                    rt.snapshot = BranchSnapshot {
                        branches: out.branches,
                        current_branch: out.current_branch,
                        polled_at: Some(now_ms()),
                        error: None,
                    };
                    rt.snapshot.clone()
                };
                self.emit(CoreEvent::BranchesUpdated { repo_id: h.id.clone(), snapshot });
            }
            Err(e) => {
                log::warn!("poll {}: {}", rc.display_name, e.message);
                let snapshot = {
                    let mut rt = h.rt();
                    rt.cache = cache;
                    match e.kind {
                        GitErrorKind::Network => rt.poll_network_backoff = true,
                        GitErrorKind::Auth => rt.poll_auth_paused = true,
                        _ => {}
                    }
                    rt.snapshot.error = Some(e);
                    rt.snapshot.clone()
                };
                self.emit(CoreEvent::BranchesUpdated { repo_id: h.id.clone(), snapshot });
            }
        }
        if had_unseen != self.has_unseen(h, &rc.branch_filter) {
            self.emit(CoreEvent::ReposChanged);
        }
    }

    fn has_unseen(&self, h: &RepoHandle, filter: &BranchFilter) -> bool {
        h.rt().snapshot.branches.iter().any(|b| !b.deleted && b.status.has_dot() && filter_matches(filter, &b.name))
    }

    fn should_tick(state: &SyncState) -> bool {
        !matches!(
            state,
            SyncState::Off
                | SyncState::Switching
                | SyncState::RemoteBranchDeleted
                | SyncState::Paused { reason: PauseReason::User }
                | SyncState::Error { error: GitError { kind: GitErrorKind::Auth, .. } }
        )
    }

    async fn sync_once(&self, h: &Arc<RepoHandle>, forced: bool) {
        let Some(rc) = self.repo_config(&h.id) else { return };
        let Some(branch) = rc.synced_branch.clone() else {
            self.set_sync_state(h, None, SyncState::Off, None);
            return;
        };
        if rc.sync_paused {
            self.set_sync_state(h, Some(branch), SyncState::Paused { reason: PauseReason::User }, None);
            return;
        }
        let state = h.rt().sync.state.clone();
        if !forced && !Self::should_tick(&state) {
            return;
        }
        let git = match self.git() {
            Ok(g) => g,
            Err(e) => {
                self.set_sync_state(h, Some(branch), SyncState::Error { error: e }, None);
                return;
            }
        };
        let _g = h.lock.lock().await;
        if !self.check_available(h) {
            return;
        }
        // Pause (or a switch) may have landed while we waited for the lock.
        if !self.still_syncing(&h.id, &branch) {
            return;
        }
        let last_synced = self.read_state(&h.id, |s| s.last_synced.get(&branch).cloned());
        let on_pull = {
            let (core, h, b) = (self.clone(), h.clone(), branch.clone());
            move || {
                if !core.still_syncing(&h.id, &b) {
                    return false;
                }
                core.set_sync_state(&h, Some(b.clone()), SyncState::Pulling, None);
                true
            }
        };
        let ctx = SyncCtx::new(&git, &h.path, &branch).last_synced(last_synced.as_deref()).policy(rc.force_push_policy);
        let ctx = SyncCtx { on_pull: Some(&on_pull), ..ctx };
        let mut res = sync::tick(ctx).await;
        // Paused mid-tick: record whatever the tick already did, but don't let
        // its result overwrite the paused state.
        if self.repo_config(&h.id).is_some_and(|c| c.sync_paused) {
            res.state = SyncState::Paused { reason: PauseReason::User };
        }
        self.apply_tick(h, &rc, &branch, res, false).await;
    }

    /// The repo is still set to sync `branch` and isn't paused by the user.
    fn still_syncing(&self, id: &str, branch: &str) -> bool {
        self.repo_config(id).is_some_and(|c| !c.sync_paused && c.synced_branch.as_deref() == Some(branch))
    }

    /// Apply a tick result: persist, flags, events, notifications.
    /// Caller holds the repo lock.
    async fn apply_tick(&self, h: &Arc<RepoHandle>, rc: &RepoConfig, branch: &str, res: TickResult, user: bool) {
        let now = now_ms();
        if let Some(sha) = &res.synced_sha {
            self.with_state(&h.id, |s| s.last_synced.insert(branch.to_string(), sha.clone()));
        }
        let network = matches!(&res.state, SyncState::Error { error } if error.kind == GitErrorKind::Network);
        h.rt().interval.set_network_backoff(network);

        if let Some(m) = &res.moved {
            h.rt().interval.reset(Instant::now());
            self.with_state(&h.id, |s| s.last_seen.insert(branch.to_string(), m.to.clone()));
            let raised = self.raise_flags(&h.id, rc, &m.files);
            let event = SyncEvent {
                branch: branch.to_string(),
                kind: SyncEventKind::Pulled {
                    from: m.from.clone(),
                    to: m.to.clone(),
                    count: m.count,
                    subject: m.subject.clone(),
                    reset: m.reset,
                },
                flags: raised.clone(),
                at: now,
            };
            self.emit(CoreEvent::SyncEvent { repo_id: h.id.clone(), event });
            if !user {
                let prefs = lock(&self.0.config).notifications.clone();
                let short = &m.to[..m.to.len().min(7)];
                let body = if m.reset {
                    prefs.force_pushed.then(|| format!("{branch} was force-pushed; reset to {short}"))
                } else {
                    prefs.pulled.then(|| {
                        let n = m.count.max(1);
                        format!("{branch} → {n} commit{}: {}", if n == 1 { "" } else { "s" }, m.subject)
                    })
                };
                if let Some(body) = body {
                    self.notify(&h.id, &rc.display_name, with_flag_text(body, &raised, rc, &h.path));
                }
            }
            self.touch_snapshot(h, Some(branch), Some((branch, &m.to)));
        }

        // Blocked / error notifications: once per occurrence.
        let block_key = match &res.state {
            s if s.is_blocked() => Some(s.kind_name().to_string()),
            SyncState::RemoteBranchDeleted => Some("remoteBranchDeleted".into()),
            SyncState::Error { error } if error.kind != GitErrorKind::Network => {
                Some(format!("error:{:?}", error.kind))
            }
            _ => None,
        };
        let newly_blocked = {
            let mut rt = h.rt();
            let newly = block_key.is_some() && rt.last_block_notified != block_key;
            if matches!(res.state, SyncState::Idle) || block_key.is_some() {
                rt.last_block_notified = block_key.clone();
            }
            newly
        };
        if newly_blocked {
            let message = res.state.describe();
            let kind = if matches!(res.state, SyncState::Error { .. }) {
                SyncEventKind::Error { message: message.clone() }
            } else {
                SyncEventKind::Blocked { message: message.clone() }
            };
            self.emit(CoreEvent::SyncEvent {
                repo_id: h.id.clone(),
                event: SyncEvent { branch: branch.to_string(), kind, flags: vec![], at: now },
            });
            if !user && lock(&self.0.config).notifications.blocked {
                self.notify(&h.id, &rc.display_name, format!("Sync of {branch} {message}"));
            }
        }
        self.set_sync_state(h, Some(branch.to_string()), res.state, res.head);
    }

    fn set_sync_state(&self, h: &RepoHandle, branch: Option<String>, state: SyncState, head: Option<String>) {
        let (status, kind_changed) = {
            let mut rt = h.rt();
            let kind_changed = rt.sync.state.kind_name() != state.kind_name() || rt.sync.branch != branch;
            if !kind_changed && rt.sync.state == state && matches!(state, SyncState::Off) {
                return;
            }
            rt.sync.branch = branch;
            rt.sync.state = state;
            rt.sync.last_checked = Some(now_ms());
            if head.is_some() {
                rt.sync.head = head;
            }
            (rt.sync.clone(), kind_changed)
        };
        self.emit(CoreEvent::SyncStatusChanged { repo_id: h.id.clone(), status });
        if kind_changed {
            self.emit(CoreEvent::ReposChanged);
        }
    }

    fn raise_flags(&self, id: &str, rc: &RepoConfig, files: &[String]) -> Vec<FlagKind> {
        if files.is_empty() {
            return vec![];
        }
        let patterns: Vec<(FlagKind, Vec<String>)> = FlagKind::ALL
            .iter()
            .map(|k| {
                let custom = rc.flag_patterns.as_ref().and_then(|p| p.get(*k)).cloned();
                (*k, custom.unwrap_or_else(|| crate::flags::default_patterns(*k)))
            })
            .collect();
        let detected = FlagMatcher::new(&patterns).detect(files);
        let kinds: Vec<FlagKind> = detected.iter().map(|(k, _)| *k).collect();
        if !detected.is_empty() {
            let flags = self.with_state(id, |s| {
                s.raise_flags(detected, now_ms());
                s.active_flags.clone()
            });
            self.emit(CoreEvent::FlagsChanged { repo_id: id.to_string(), flags });
        }
        kinds
    }

    /// Update the in-memory branch rows after a sync / switch / stop without
    /// touching git, then ask the poll loop to refresh from the remote.
    fn touch_snapshot(&self, h: &RepoHandle, synced: Option<&str>, seen: Option<(&str, &str)>) {
        let snapshot = {
            let mut rt = h.rt();
            for b in rt.snapshot.branches.iter_mut() {
                b.is_synced = synced == Some(b.name.as_str());
                if let Some((name, sha)) = seen {
                    if b.name == name && b.sha == sha {
                        b.status = BranchStatus::Seen;
                    }
                }
            }
            if seen.is_some() {
                rt.snapshot.current_branch = synced.map(String::from).or(rt.snapshot.current_branch.take());
                let current = rt.snapshot.current_branch.clone();
                for b in rt.snapshot.branches.iter_mut() {
                    b.is_current = current.as_deref() == Some(b.name.as_str());
                }
            }
            rt.snapshot.clone()
        };
        self.emit(CoreEvent::BranchesUpdated { repo_id: h.id.clone(), snapshot });
        self.emit(CoreEvent::ReposChanged);
        h.wake_poll.notify_one();
    }

    fn notify(&self, repo_id: &str, title: &str, body: String) {
        let note = Note { title: title.to_string(), body };
        let offer = lock(&self.0.limiter).offer(repo_id, note, Instant::now());
        match offer {
            Offer::Send(n) => self.emit(CoreEvent::Notify { repo_id: repo_id.into(), title: n.title, body: n.body }),
            Offer::Queued => {}
            Offer::ScheduleFlush(delay) => {
                let (core, id) = (self.clone(), repo_id.to_string());
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let n = lock(&core.0.limiter).flush(&id, Instant::now());
                    if let Some(n) = n {
                        core.emit(CoreEvent::Notify { repo_id: id, title: n.title, body: n.body });
                    }
                });
            }
        }
    }

    // -----------------------------------------------------------------------
    // Public operations (IPC surface)
    // -----------------------------------------------------------------------

    pub fn app_info(&self) -> AppInfo {
        let git = self.git();
        AppInfo {
            version: self.0.app_version.clone(),
            git_path: git.as_ref().ok().map(|g| g.path().display().to_string()),
            git_version: git.as_ref().ok().map(|g| g.version_string()),
            git_error: git.err().map(|e| e.message),
        }
    }

    fn summary(&self, rc: &RepoConfig) -> RepoSummary {
        let h = lock(&self.0.repos).get(&rc.id).cloned();
        let (available, sync, has_unseen) = match &h {
            Some(h) => {
                let has_unseen = self.has_unseen(h, &rc.branch_filter);
                let rt = h.rt();
                (rt.available, rt.sync.clone(), has_unseen)
            }
            None => (false, SyncStatus { branch: None, state: SyncState::Off, last_checked: None, head: None }, false),
        };
        let path = PathBuf::from(&rc.path);
        RepoSummary {
            id: rc.id.clone(),
            config: rc.clone(),
            available,
            has_unseen,
            sync,
            flags: self.read_state(&rc.id, |s| s.active_flags.clone()),
            install_command: rc.install_command.clone().or_else(|| detect_install_command(&path)),
        }
    }

    pub fn list_repos(&self) -> Vec<RepoSummary> {
        let repos = lock(&self.0.config).repos.clone();
        repos.iter().map(|rc| self.summary(rc)).collect()
    }

    pub async fn add_repo(&self, path: &str) -> CmdResult<RepoSummary> {
        let git = self.git()?;
        let v = repo::validate(&git, Path::new(path)).await?;
        let top = v.toplevel.display().to_string();
        if let Some(existing) = lock(&self.0.config).repos.iter().find(|r| r.path == top) {
            return Err(CmdError::new(CmdErrorKind::Invalid, format!("{} is already added", existing.display_name)));
        }
        let name = v.toplevel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| top.clone());
        let rc = RepoConfig::new(uuid::Uuid::new_v4().to_string(), top, name);
        {
            let mut c = lock(&self.0.config);
            c.repos.push(rc.clone());
            self.save_config_locked(&c);
        }
        // Seed the seen-state before the loops start, so existing branches
        // don't all light up as "new".
        let h = self.make_handle(&rc);
        self.poll_once(&h, true).await;
        self.start_loops(&h);
        self.emit(CoreEvent::ReposChanged);
        Ok(self.summary(&rc))
    }

    pub fn remove_repo(&self, id: &str) -> CmdResult<()> {
        if let Some(h) = lock(&self.0.repos).remove(id) {
            let _ = h.cancel.send(true);
        }
        {
            let mut c = lock(&self.0.config);
            c.repos.retain(|r| r.id != id);
            self.save_config_locked(&c);
        }
        lock(&self.0.state).repos.remove(id);
        self.0.state_dirty.notify_one();
        self.emit(CoreEvent::ReposChanged);
        Ok(())
    }

    /// Point a repo at a new folder (after it was moved on disk).
    pub async fn locate_repo(&self, id: &str, path: &str) -> CmdResult<RepoSummary> {
        let git = self.git()?;
        let v = repo::validate(&git, Path::new(path)).await?;
        let top = v.toplevel.display().to_string();
        let rc = self
            .with_repo_config(id, |r| {
                r.path = top.clone();
                r.clone()
            })
            .ok_or_else(|| CmdError::not_found("repo"))?;
        if let Some(h) = lock(&self.0.repos).remove(id) {
            let _ = h.cancel.send(true);
        }
        self.spawn_watcher(&rc);
        self.emit(CoreEvent::ReposChanged);
        Ok(self.summary(&rc))
    }

    /// Patch user-editable repo fields. `null` clears an optional field.
    pub fn update_repo_config(&self, id: &str, patch: Value) -> CmdResult<RepoSummary> {
        const EDITABLE: &[&str] = &[
            "displayName",
            "pollingEnabled",
            "branchFilter",
            "forcePushPolicy",
            "installCommand",
            "migrateCommand",
            "flagPatterns",
        ];
        let rc = self.repo_config(id).ok_or_else(|| CmdError::not_found("repo"))?;
        let merged: RepoConfig = merge_patch(&rc, &patch, EDITABLE)?;
        self.with_repo_config(id, |r| *r = merged.clone());
        if let Ok(h) = self.handle(id) {
            h.wake_poll.notify_one();
        }
        self.emit(CoreEvent::ReposChanged);
        Ok(self.summary(&merged))
    }

    pub fn get_branches(&self, id: &str) -> CmdResult<BranchSnapshot> {
        Ok(self.handle(id)?.rt().snapshot.clone())
    }

    pub fn get_sync_status(&self, id: &str) -> CmdResult<SyncStatus> {
        Ok(self.handle(id)?.rt().sync.clone())
    }

    /// Mark one branch (or all) as seen. No git needed.
    pub fn mark_seen(&self, id: &str, branch: Option<&str>) -> CmdResult<()> {
        let h = self.handle(id)?;
        let rc = self.repo_config(id).ok_or_else(|| CmdError::not_found("repo"))?;
        let tips: Vec<(String, String)> = h
            .rt()
            .snapshot
            .branches
            .iter()
            .filter(|b| !b.deleted && branch.is_none_or(|n| n == b.name))
            .map(|b| (b.name.clone(), b.sha.clone()))
            .collect();
        self.with_state(id, |s| s.last_seen.extend(tips.iter().cloned()));
        let snapshot = {
            let mut rt = h.rt();
            for b in rt.snapshot.branches.iter_mut() {
                if tips.iter().any(|(n, _)| *n == b.name) {
                    b.status = BranchStatus::Seen;
                }
            }
            rt.snapshot.clone()
        };
        let _ = rc;
        self.emit(CoreEvent::BranchesUpdated { repo_id: id.into(), snapshot });
        self.emit(CoreEvent::ReposChanged);
        Ok(())
    }

    pub async fn get_branch_log(&self, id: &str, branch: &str, n: usize) -> CmdResult<Vec<CommitSummary>> {
        let h = self.handle(id)?;
        let git = self.git()?;
        let r = format!("refs/remotes/origin/{branch}");
        Ok(repo::log(&git, &h.path, &r, n.clamp(1, 100)).await?)
    }

    /// Switch the checkout to `branch` and start syncing it.
    pub async fn start_sync(&self, id: &str, branch: &str, discard: bool) -> CmdResult<SyncStatus> {
        let h = self.handle(id)?;
        let rc = self.repo_config(id).ok_or_else(|| CmdError::not_found("repo"))?;
        let git = self.git()?;
        let prev = h.rt().sync.clone();
        self.set_sync_state(&h, Some(branch.to_string()), SyncState::Switching, None);

        let _g = h.lock.lock().await;
        let last_synced = self.read_state(id, |s| s.last_synced.get(branch).cloned());
        let ctx = SyncCtx::new(&git, &h.path, branch).last_synced(last_synced.as_deref()).policy(rc.force_push_policy);
        let res = sync::switch(ctx, discard).await;
        let r = match res {
            Ok(r) => r,
            Err(e) => {
                self.set_sync_state(&h, prev.branch, prev.state, None);
                return Err(match e {
                    SwitchError::Dirty(paths) => CmdError {
                        kind: CmdErrorKind::Dirty,
                        message: "Local changes would be lost by switching".into(),
                        paths,
                    },
                    SwitchError::UntrackedInTheWay(paths) => CmdError {
                        kind: CmdErrorKind::UntrackedInTheWay,
                        message: "Untracked files would be overwritten by switching".into(),
                        paths,
                    },
                    SwitchError::Git(e) => e.into(),
                });
            }
        };

        let rc = self
            .with_repo_config(id, |c| {
                c.synced_branch = Some(branch.to_string());
                c.sync_paused = false;
                c.clone()
            })
            .unwrap_or(rc);
        h.rt().interval.reset(Instant::now());
        if let Some(sha) = &r.tick.synced_sha {
            self.with_state(id, |s| {
                s.last_synced.insert(branch.to_string(), sha.clone());
                s.last_seen.insert(branch.to_string(), sha.clone());
            });
        }
        let raised = self.raise_flags(id, &rc, &r.files);
        self.emit(CoreEvent::SyncEvent {
            repo_id: id.into(),
            event: SyncEvent {
                branch: branch.into(),
                kind: SyncEventKind::Switched {
                    from: r.from_branch.clone(),
                    to: branch.into(),
                    sha: r.new_head.clone().unwrap_or_default(),
                },
                flags: raised,
                at: now_ms(),
            },
        });
        h.rt().last_block_notified = None;
        let tick = TickResult { moved: None, synced_sha: None, ..r.tick };
        self.apply_tick(&h, &rc, branch, tick, true).await;
        if let Some(head) = &r.new_head {
            self.touch_snapshot(&h, Some(branch), Some((branch, head)));
        }
        drop(_g);
        h.wake_sync.notify_one();
        let status = h.rt().sync.clone();
        Ok(status)
    }

    pub fn stop_sync(&self, id: &str) -> CmdResult<SyncStatus> {
        let h = self.handle(id)?;
        self.with_repo_config(id, |c| {
            c.synced_branch = None;
            c.sync_paused = false;
        });
        self.set_sync_state(&h, None, SyncState::Off, None);
        self.touch_snapshot(&h, None, None);
        let status = h.rt().sync.clone();
        Ok(status)
    }

    pub fn pause_sync(&self, id: &str) -> CmdResult<SyncStatus> {
        let h = self.handle(id)?;
        let branch = self.with_repo_config(id, |c| {
            c.sync_paused = true;
            c.synced_branch.clone()
        });
        self.set_sync_state(&h, branch.flatten(), SyncState::Paused { reason: PauseReason::User }, None);
        let status = h.rt().sync.clone();
        Ok(status)
    }

    pub async fn resume_sync(&self, id: &str) -> CmdResult<SyncStatus> {
        self.with_repo_config(id, |c| c.sync_paused = false);
        self.sync_now(id).await
    }

    /// Check right away, clearing any auth/network back-off (the Retry button).
    pub async fn sync_now(&self, id: &str) -> CmdResult<SyncStatus> {
        let h = self.handle(id)?;
        if self.git().is_err() {
            self.rediscover_git().await;
        }
        {
            let mut rt = h.rt();
            rt.poll_auth_paused = false;
            rt.poll_network_backoff = false;
            rt.interval.reset(Instant::now());
            rt.interval.set_network_backoff(false);
            rt.cache.invalidate();
        }
        h.wake_poll.notify_one();
        self.sync_once(&h, true).await;
        let status = h.rt().sync.clone();
        Ok(status)
    }

    pub async fn discard_changes(&self, id: &str) -> CmdResult<SyncStatus> {
        let h = self.handle(id)?;
        let git = self.git()?;
        {
            let _g = h.lock.lock().await;
            sync::discard_changes(&git, &h.path).await?;
        }
        self.sync_now(id).await
    }

    pub async fn move_untracked_aside(&self, id: &str, paths: &[String]) -> CmdResult<Vec<String>> {
        let h = self.handle(id)?;
        let git = self.git()?;
        let moved = {
            let _g = h.lock.lock().await;
            sync::move_untracked_aside(&git, &h.path, paths, now_ms() / 1000).await?
        };
        if self.repo_config(id).and_then(|r| r.synced_branch).is_some() {
            self.sync_now(id).await?;
        }
        Ok(moved)
    }

    pub async fn reset_to_remote(&self, id: &str) -> CmdResult<SyncStatus> {
        let h = self.handle(id)?;
        let rc = self.repo_config(id).ok_or_else(|| CmdError::not_found("repo"))?;
        let branch =
            rc.synced_branch.clone().ok_or_else(|| CmdError::new(CmdErrorKind::Invalid, "No synced branch"))?;
        let git = self.git()?;
        let _g = h.lock.lock().await;
        let ctx = SyncCtx::new(&git, &h.path, &branch).policy(rc.force_push_policy);
        let res = sync::reset_to_remote(ctx).await?;
        h.rt().last_block_notified = None;
        self.apply_tick(&h, &rc, &branch, res, true).await;
        let status = h.rt().sync.clone();
        Ok(status)
    }

    pub fn dismiss_flag(&self, id: &str, kind: FlagKind) -> CmdResult<Vec<ActiveFlag>> {
        let flags = self.with_state(id, |s| {
            s.dismiss_flag(kind);
            s.active_flags.clone()
        });
        self.emit(CoreEvent::FlagsChanged { repo_id: id.into(), flags: flags.clone() });
        Ok(flags)
    }

    pub fn get_settings(&self) -> Config {
        let mut c = lock(&self.0.config).clone();
        c.repos.clear();
        c
    }

    /// Patch global settings. Returns the new settings.
    pub async fn update_settings(&self, patch: Value) -> CmdResult<Config> {
        const EDITABLE: &[&str] =
            &["gitPath", "launchAtLogin", "startHidden", "branchPollSeconds", "syncBaseSeconds", "notifications"];
        let current = lock(&self.0.config).clone();
        let mut merged: Config = merge_patch(&current, &patch, EDITABLE)?;
        merged.branch_poll_seconds = merged.branch_poll_seconds.clamp(5, 3600);
        merged.sync_base_seconds = merged.sync_base_seconds.clamp(1, 600);
        if merged.git_path.as_deref().is_some_and(|p| p.trim().is_empty()) {
            merged.git_path = None;
        }
        let git_changed = merged.git_path != current.git_path;
        {
            let mut c = lock(&self.0.config);
            let repos = std::mem::take(&mut c.repos);
            *c = Config { repos, ..merged };
            self.save_config_locked(&c);
        }
        if git_changed {
            self.rediscover_git().await;
        }
        self.wake_all(true);
        self.emit(CoreEvent::SettingsChanged);
        Ok(self.get_settings())
    }

    async fn rediscover_git(&self) {
        let path = lock(&self.0.config).git_path.clone();
        let git = Git::discover(path.as_deref().map(Path::new)).await;
        *self.0.git.write().unwrap_or_else(|p| p.into_inner()) = git;
    }

    fn wake_all(&self, reset_interval: bool) {
        let handles: Vec<Arc<RepoHandle>> = lock(&self.0.repos).values().cloned().collect();
        for h in handles {
            if reset_interval {
                h.rt().interval.reset(Instant::now());
            }
            h.wake_poll.notify_one();
            h.wake_sync.notify_one();
        }
    }

    /// The app window gained focus: poll now and return to the fast interval.
    pub fn on_focus(&self) {
        self.wake_all(true);
    }

    /// The machine woke from sleep.
    pub fn on_wake(&self) {
        let handles: Vec<Arc<RepoHandle>> = lock(&self.0.repos).values().cloned().collect();
        for h in &handles {
            let mut rt = h.rt();
            rt.poll_network_backoff = false;
            rt.interval.set_network_backoff(false);
        }
        self.wake_all(true);
    }

    pub fn tray_model(&self) -> TrayModel {
        let repos = lock(&self.0.config).repos.clone();
        let mut synced = Vec::new();
        let mut unseen: Vec<(i64, TrayUnseen)> = Vec::new();
        let mut icon = TrayIconState::Idle;
        let mut bump = |s: TrayIconState| {
            let rank = |s: TrayIconState| match s {
                TrayIconState::Idle => 0,
                TrayIconState::Unseen => 1,
                TrayIconState::Syncing => 2,
                TrayIconState::Blocked => 3,
                TrayIconState::Error => 4,
            };
            if rank(s) > rank(icon) {
                icon = s;
            }
        };
        for rc in &repos {
            let Some(h) = lock(&self.0.repos).get(&rc.id).cloned() else { continue };
            let rt = h.rt();
            if let Some(b) = &rt.sync.branch {
                let st = &rt.sync.state;
                bump(match st {
                    SyncState::Error { .. } => TrayIconState::Error,
                    s if s.is_blocked() => TrayIconState::Blocked,
                    SyncState::Off => TrayIconState::Idle,
                    _ => TrayIconState::Syncing,
                });
                synced.push(TraySynced {
                    repo_id: rc.id.clone(),
                    repo_name: rc.display_name.clone(),
                    branch: b.clone(),
                    description: st.describe(),
                    paused: matches!(st, SyncState::Paused { reason: PauseReason::User }),
                });
            }
            for br in rt.snapshot.branches.iter() {
                if br.deleted || br.is_synced || !br.status.has_dot() || !filter_matches(&rc.branch_filter, &br.name) {
                    continue;
                }
                bump(TrayIconState::Unseen);
                let label = match &br.status {
                    BranchStatus::New => "new".to_string(),
                    BranchStatus::Unseen { count } => format!("{count} new"),
                    BranchStatus::Rewritten => "rewritten".to_string(),
                    BranchStatus::Seen => String::new(),
                };
                unseen.push((
                    br.committed_at,
                    TrayUnseen {
                        repo_id: rc.id.clone(),
                        repo_name: rc.display_name.clone(),
                        branch: br.name.clone(),
                        label,
                    },
                ));
            }
        }
        unseen.sort_by_key(|u| std::cmp::Reverse(u.0));
        TrayModel { icon, synced, unseen: unseen.into_iter().take(5).map(|(_, u)| u).collect() }
    }

    /// Stop all watchers (used by tests and on quit).
    pub fn shutdown(&self) {
        for h in lock(&self.0.repos).values() {
            let _ = h.cancel.send(true);
        }
        self.flush();
    }
}

fn with_flag_text(mut body: String, flags: &[FlagKind], rc: &RepoConfig, path: &Path) -> String {
    for f in flags {
        let cmd = match f {
            FlagKind::Deps => rc.install_command.clone().or_else(|| detect_install_command(path)),
            FlagKind::Migrations => rc.migrate_command.clone(),
            FlagKind::Env => None,
        };
        body.push('\n');
        body.push_str(f.label());
        match (f, cmd) {
            (_, Some(c)) => body.push_str(&format!(": run {c}")),
            (FlagKind::Deps, None) => body.push_str(": run install"),
            (FlagKind::Migrations, None) => body.push_str(": run migrate"),
            (FlagKind::Env, None) => body.push_str(": check your .env"),
        }
    }
    body
}

/// Apply a JSON merge patch restricted to `editable` top-level keys.
fn merge_patch<T: Serialize + for<'de> serde::Deserialize<'de>>(
    current: &T,
    patch: &Value,
    editable: &[&str],
) -> CmdResult<T> {
    let Value::Object(patch) = patch else {
        return Err(CmdError::new(CmdErrorKind::Invalid, "patch must be an object"));
    };
    let mut v = serde_json::to_value(current).map_err(|e| CmdError::new(CmdErrorKind::Invalid, e.to_string()))?;
    let obj = v.as_object_mut().expect("structs serialise to objects");
    for (k, val) in patch {
        if !editable.contains(&k.as_str()) {
            return Err(CmdError::new(CmdErrorKind::Invalid, format!("{k} cannot be changed here")));
        }
        if val.is_null() {
            obj.remove(k);
        } else {
            obj.insert(k.clone(), val.clone());
        }
    }
    serde_json::from_value(v).map_err(|e| CmdError::new(CmdErrorKind::Invalid, format!("invalid settings: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_patch_rules() {
        let rc = RepoConfig::new("id".into(), "/p".into(), "p".into());
        let out: RepoConfig = merge_patch(
            &rc,
            &serde_json::json!({"displayName": "x", "installCommand": "pnpm i"}),
            &["displayName", "installCommand"],
        )
        .unwrap();
        assert_eq!(out.display_name, "x");
        assert_eq!(out.install_command.as_deref(), Some("pnpm i"));
        let out: RepoConfig =
            merge_patch(&out, &serde_json::json!({"installCommand": null}), &["installCommand"]).unwrap();
        assert_eq!(out.install_command, None);
        assert!(merge_patch::<RepoConfig>(&rc, &serde_json::json!({"path": "/etc"}), &["displayName"]).is_err());
        assert!(
            merge_patch::<RepoConfig>(&rc, &serde_json::json!({"pollingEnabled": "yes"}), &["pollingEnabled"]).is_err()
        );
    }

    #[test]
    fn filter() {
        let f = BranchFilter::default();
        assert!(filter_matches(&f, "claude/x"));
        assert!(!filter_matches(&f, "main"));
        assert!(filter_matches(&BranchFilter { mode: FilterMode::All, prefix: None }, "main"));
    }
}

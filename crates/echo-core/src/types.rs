//! Types shared with the UI over IPC. All serialise as camelCase JSON.

use crate::devserver::DevServerStatus;
use crate::flags::FlagKind;
use crate::git::GitError;
use crate::store::{ActiveFlag, RepoConfig};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BranchStatus {
    /// Tip equals the last-seen sha.
    Seen,
    /// No last-seen sha: branch appeared since the user last looked.
    New,
    /// Fast-forward of last-seen: `count` unseen commits.
    Unseen { count: u32 },
    /// Last-seen is not an ancestor: force-pushed.
    Rewritten,
}

impl BranchStatus {
    pub fn has_dot(&self) -> bool {
        !matches!(self, BranchStatus::Seen)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub sha: String,
    pub subject: String,
    pub author: String,
    /// Unix seconds.
    pub committed_at: i64,
    pub status: BranchStatus,
    /// Checked out locally (HEAD points at this branch name).
    pub is_current: bool,
    /// The repo's synced branch.
    pub is_synced: bool,
    /// Gone from the remote; shown greyed for one poll cycle.
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub committed_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PauseReason {
    /// User pressed Pause.
    User,
    /// HEAD is no longer on the synced branch.
    #[serde(rename_all = "camelCase")]
    CheckoutChanged { current: Option<String> },
    /// A rebase / merge / cherry-pick is in progress; clears by itself.
    GitOperation,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SyncState {
    Off,
    Switching,
    Idle,
    Pulling,
    Paused {
        reason: PauseReason,
    },
    /// Tracked files modified, or untracked files in the way.
    #[serde(rename_all = "camelCase")]
    BlockedDirty {
        files: Vec<String>,
        untracked: Vec<String>,
        /// Every modified file differs only in line endings.
        crlf_hint: bool,
    },
    BlockedLocalCommits {
        commits: Vec<CommitSummary>,
    },
    /// Remote rewritten and local has its own commits.
    BlockedDiverged {
        commits: Vec<CommitSummary>,
    },
    /// Remote rewritten, local is clean, policy is "ask".
    #[serde(rename_all = "camelCase")]
    BlockedForcePush {
        remote_sha: String,
    },
    RemoteBranchDeleted,
    Error {
        error: GitError,
    },
}

impl SyncState {
    pub fn kind_name(&self) -> &'static str {
        match self {
            SyncState::Off => "off",
            SyncState::Switching => "switching",
            SyncState::Idle => "idle",
            SyncState::Pulling => "pulling",
            SyncState::Paused { .. } => "paused",
            SyncState::BlockedDirty { .. } => "blockedDirty",
            SyncState::BlockedLocalCommits { .. } => "blockedLocalCommits",
            SyncState::BlockedDiverged { .. } => "blockedDiverged",
            SyncState::BlockedForcePush { .. } => "blockedForcePush",
            SyncState::RemoteBranchDeleted => "remoteBranchDeleted",
            SyncState::Error { .. } => "error",
        }
    }

    pub fn is_blocked(&self) -> bool {
        matches!(
            self,
            SyncState::BlockedDirty { .. }
                | SyncState::BlockedLocalCommits { .. }
                | SyncState::BlockedDiverged { .. }
                | SyncState::BlockedForcePush { .. }
        )
    }

    /// Short human description, used for tray and notifications.
    pub fn describe(&self) -> String {
        match self {
            SyncState::Off => "not syncing".into(),
            SyncState::Switching => "switching…".into(),
            SyncState::Idle => "up to date".into(),
            SyncState::Pulling => "pulling…".into(),
            SyncState::Paused { reason: PauseReason::User } => "paused".into(),
            SyncState::Paused { reason: PauseReason::CheckoutChanged { current } } => {
                format!("paused: checkout changed to {}", current.as_deref().unwrap_or("a detached HEAD"))
            }
            SyncState::Paused { reason: PauseReason::GitOperation } => "waiting: git operation in progress".into(),
            SyncState::BlockedDirty { files, untracked, .. } => {
                format!("blocked: {} local change(s)", files.len() + untracked.len())
            }
            SyncState::BlockedLocalCommits { commits } => format!("blocked: {} local commit(s)", commits.len()),
            SyncState::BlockedDiverged { .. } => "blocked: remote rewritten and local has its own commits".into(),
            SyncState::BlockedForcePush { .. } => "blocked: remote was force-pushed".into(),
            SyncState::RemoteBranchDeleted => "branch deleted on remote (merged?)".into(),
            SyncState::Error { error } => format!("error: {}", error.message),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub branch: Option<String>,
    pub state: SyncState,
    /// Unix millis of the last completed check.
    pub last_checked: Option<i64>,
    pub head: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SyncEventKind {
    #[serde(rename_all = "camelCase")]
    Pulled {
        from: String,
        to: String,
        count: u32,
        subject: String,
        reset: bool,
    },
    #[serde(rename_all = "camelCase")]
    Switched {
        from: Option<String>,
        to: String,
        sha: String,
    },
    Blocked {
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncEvent {
    pub branch: String,
    #[serde(flatten)]
    pub kind: SyncEventKind,
    pub flags: Vec<FlagKind>,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BranchSnapshot {
    pub branches: Vec<BranchInfo>,
    pub current_branch: Option<String>,
    pub polled_at: Option<i64>,
    pub error: Option<GitError>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoSummary {
    pub id: String,
    pub config: RepoConfig,
    pub available: bool,
    pub has_unseen: bool,
    pub sync: SyncStatus,
    pub flags: Vec<ActiveFlag>,
    /// Install command (configured, or detected from lockfiles).
    pub install_command: Option<String>,
    /// `https://github.com/<owner>/<repo>` when origin is on GitHub.
    pub github_url: Option<String>,
    pub dev_server: DevServerStatus,
    /// Dev command guessed from package.json, offered when nothing has been run.
    pub suggested_dev_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub git_path: Option<String>,
    pub git_version: Option<String>,
    pub git_error: Option<String>,
    /// GitHub CLI, used to list pull requests. `None` when not installed.
    pub gh_path: Option<String>,
}

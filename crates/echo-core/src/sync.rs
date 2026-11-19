//! The sync state machine: one tick of the sync loop, switching the synced
//! branch, and the explicit blocked-state fixes.
//!
//! These functions do git work and return the resulting [`SyncState`]; they
//! never touch persistence or emit events (that's the watcher's job), which
//! keeps them easy to drive from integration tests.

use crate::git::{Git, GitError, GitErrorKind, Mode};
use crate::repo;
use crate::store::ForcePushPolicy;
use crate::types::{PauseReason, SyncState};
use std::path::Path;

#[derive(Clone, Copy)]
pub struct SyncCtx<'a> {
    pub git: &'a Git,
    pub repo: &'a Path,
    pub branch: &'a str,
    /// Last sha the app put this branch at.
    pub last_synced: Option<&'a str>,
    pub policy: ForcePushPolicy,
    /// Called just before a tick starts fetching (the remote moved), so the UI
    /// can show "Pulling".
    pub on_pull: Option<&'a (dyn Fn() + Send + Sync)>,
}

impl<'a> SyncCtx<'a> {
    pub fn new(git: &'a Git, repo: &'a Path, branch: &'a str) -> Self {
        Self { git, repo, branch, last_synced: None, policy: ForcePushPolicy::Reset, on_pull: None }
    }
    pub fn last_synced(mut self, sha: Option<&'a str>) -> Self {
        self.last_synced = sha;
        self
    }
    pub fn policy(mut self, policy: ForcePushPolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// A HEAD move performed by the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub from: String,
    pub to: String,
    /// Number of commits in `to` that weren't in `from`.
    pub count: u32,
    /// Subject of the new tip.
    pub subject: String,
    /// True for a `reset --hard` (force-push), false for a fast-forward.
    pub reset: bool,
    /// Files that differ between `from` and `to`.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TickResult {
    pub state: SyncState,
    /// HEAD after the tick, if known.
    pub head: Option<String>,
    pub moved: Option<Moved>,
    /// New value for `lastSynced[branch]`, when HEAD is known to equal the remote.
    pub synced_sha: Option<String>,
}

impl TickResult {
    fn state(state: SyncState, head: Option<String>) -> Self {
        Self { state, head, moved: None, synced_sha: None }
    }
}

fn remote_ref(branch: &str) -> String {
    format!("refs/remotes/origin/{branch}")
}

fn branch_refspec(branch: &str) -> String {
    format!("+refs/heads/{branch}:refs/remotes/origin/{branch}")
}

/// Map an error into the state the UI should show.
fn error_state(e: GitError) -> SyncState {
    match e.kind {
        GitErrorKind::RemoteGone => SyncState::RemoteBranchDeleted,
        GitErrorKind::WouldOverwrite { paths, untracked: true } => {
            SyncState::BlockedDirty { files: vec![], untracked: paths, crlf_hint: false }
        }
        GitErrorKind::WouldOverwrite { paths, untracked: false } => {
            SyncState::BlockedDirty { files: paths, untracked: vec![], crlf_hint: false }
        }
        _ => SyncState::Error { error: e },
    }
}

/// One tick of the sync loop for the synced branch.
pub async fn tick(ctx: SyncCtx<'_>) -> TickResult {
    match tick_inner(ctx).await {
        Ok(r) => r,
        Err(e) => TickResult::state(error_state(e), None),
    }
}

async fn tick_inner(ctx: SyncCtx<'_>) -> Result<TickResult, GitError> {
    let SyncCtx { git, repo, branch, .. } = ctx;
    if repo::operation_in_progress(git, repo).await? {
        return Ok(TickResult::state(SyncState::Paused { reason: PauseReason::GitOperation }, None));
    }
    let current = repo::head_branch(git, repo).await?;
    if current.as_deref() != Some(branch) {
        return Ok(TickResult::state(SyncState::Paused { reason: PauseReason::CheckoutChanged { current } }, None));
    }
    let head = repo::head_sha(git, repo).await?;

    let refname = format!("refs/heads/{branch}");
    let out = git.run(repo, &["ls-remote", "origin", &refname], Mode::Network).await?;
    let remote_sha = crate::git::parse_ls_remote(&out)
        .into_iter()
        .find(|(n, _)| n == branch)
        .map(|(_, s)| s);
    let Some(remote_sha) = remote_sha else {
        return Ok(TickResult::state(SyncState::RemoteBranchDeleted, head));
    };
    if head.as_deref() == Some(remote_sha.as_str()) {
        // Up to date. Recheck dirtiness only if cheap and useful: a clean,
        // up-to-date checkout is Idle; local edits don't matter until a pull.
        return Ok(TickResult { state: SyncState::Idle, synced_sha: head.clone(), head, moved: None });
    }

    if let Some(f) = ctx.on_pull {
        f();
    }
    git.run(repo, &["fetch", "--no-tags", "origin", &branch_refspec(branch)], Mode::Network)
        .await?;
    decide(ctx, false).await
}

/// Steps 5–7 of the sync loop: dirty check, ancestry decision, fast-forward or
/// reset. `force_reset` skips the "ask" policy (user confirmed).
pub async fn decide(ctx: SyncCtx<'_>, force_reset: bool) -> Result<TickResult, GitError> {
    let SyncCtx { git, repo, branch, last_synced, policy, .. } = ctx;
    let target_ref = remote_ref(branch);

    for _attempt in 0..2 {
        let dirty = repo::dirty_files(git, repo).await?;
        if !dirty.is_empty() {
            let crlf_hint = repo::dirty_only_line_endings(git, repo, &dirty).await;
            return Ok(TickResult::state(
                SyncState::BlockedDirty { files: dirty, untracked: vec![], crlf_hint },
                None,
            ));
        }
        let Some(target) = repo::rev_parse(git, repo, &target_ref).await? else {
            return Ok(TickResult::state(SyncState::RemoteBranchDeleted, None));
        };
        let Some(head) = repo::head_sha(git, repo).await? else {
            return Err(GitError::other("HEAD has no commits"));
        };
        if head == target {
            return Ok(TickResult { state: SyncState::Idle, head: Some(head.clone()), moved: None, synced_sha: Some(head) });
        }

        if repo::is_ancestor(git, repo, &head, &target).await? {
            match git.run(repo, &["merge", "--ff-only", "--no-stat", &target], Mode::Write).await {
                Ok(_) => {}
                Err(e) if e.kind == GitErrorKind::NotFastForward => continue,
                Err(e) => return Err(e),
            }
            let moved = describe_move(git, repo, &head, &target, false).await?;
            return Ok(TickResult { state: SyncState::Idle, head: Some(target.clone()), moved: Some(moved), synced_sha: Some(target) });
        }

        if repo::is_ancestor(git, repo, &target, &head).await? {
            let commits = repo::log(git, repo, &format!("{target}..{head}"), 20).await?;
            return Ok(TickResult::state(SyncState::BlockedLocalCommits { commits }, Some(head)));
        }

        // Diverged.
        if last_synced == Some(head.as_str()) {
            if policy == ForcePushPolicy::Ask && !force_reset {
                return Ok(TickResult::state(SyncState::BlockedForcePush { remote_sha: target }, Some(head)));
            }
            return reset_hard(git, repo, &head, &target).await;
        }
        let commits = repo::log(git, repo, &format!("{target}..{head}"), 20).await?;
        return Ok(TickResult::state(SyncState::BlockedDiverged { commits }, Some(head)));
    }
    Err(GitError::new(GitErrorKind::NotFastForward, "could not fast-forward after retrying"))
}

async fn reset_hard(git: &Git, repo: &Path, head: &str, target: &str) -> Result<TickResult, GitError> {
    let in_the_way = repo::untracked_in_the_way(git, repo, target).await?;
    if !in_the_way.is_empty() {
        return Ok(TickResult::state(
            SyncState::BlockedDirty { files: vec![], untracked: in_the_way, crlf_hint: false },
            Some(head.to_string()),
        ));
    }
    git.run(repo, &["reset", "--hard", "--quiet", target], Mode::Write).await?;
    let moved = describe_move(git, repo, head, target, true).await?;
    Ok(TickResult { state: SyncState::Idle, head: Some(target.to_string()), moved: Some(moved), synced_sha: Some(target.to_string()) })
}

async fn describe_move(git: &Git, repo: &Path, from: &str, to: &str, reset: bool) -> Result<Moved, GitError> {
    let count = repo::count(git, repo, &format!("{from}..{to}")).await.unwrap_or(0);
    let subject = repo::log(git, repo, to, 1)
        .await
        .ok()
        .and_then(|v| v.into_iter().next())
        .map(|c| c.subject)
        .unwrap_or_default();
    let files = repo::changed_files(git, repo, from, to).await.unwrap_or_default();
    Ok(Moved { from: from.to_string(), to: to.to_string(), count, subject, reset, files })
}

/// Explicit user action: discard tracked changes (`reset --hard HEAD`).
/// Untracked files are left alone.
pub async fn discard_changes(git: &Git, repo: &Path) -> Result<(), GitError> {
    git.run(repo, &["reset", "--hard", "--quiet", "HEAD"], Mode::Write).await.map(|_| ())
}

/// Explicit user action: rename untracked files to `<name>.echo-<timestamp>`.
/// Returns the new names. Refuses tracked files and paths outside the repo.
pub async fn move_untracked_aside(git: &Git, repo: &Path, paths: &[String], stamp: i64) -> Result<Vec<String>, GitError> {
    let mut moved = Vec::new();
    for p in paths {
        let rel = Path::new(p);
        if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(GitError::other(format!("refusing to move {p}: not a path inside the repo")));
        }
        let tracked = git.check(repo, &["ls-files", "--error-unmatch", "--", p]).await.unwrap_or(false);
        if tracked {
            return Err(GitError::other(format!("refusing to move {p}: it is tracked by git")));
        }
        let src = repo.join(rel);
        if src.symlink_metadata().is_err() {
            continue;
        }
        let name = format!("{}.echo-{stamp}", rel.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
        let dst = src.with_file_name(&name);
        std::fs::rename(&src, &dst).map_err(|e| GitError::other(format!("could not rename {p}: {e}")))?;
        let new_rel = match rel.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => format!("{}/{}", parent.to_string_lossy(), name),
            _ => name,
        };
        moved.push(new_rel);
    }
    Ok(moved)
}

/// Explicit user action: fetch and `reset --hard origin/<branch>`.
pub async fn reset_to_remote(ctx: SyncCtx<'_>) -> Result<TickResult, GitError> {
    let SyncCtx { git, repo, branch, .. } = ctx;
    if repo::head_branch(git, repo).await?.as_deref() != Some(branch) {
        return Err(GitError::other(format!("{branch} is not checked out")));
    }
    git.run(repo, &["fetch", "--no-tags", "origin", &branch_refspec(branch)], Mode::Network)
        .await?;
    let target = repo::rev_parse(git, repo, &remote_ref(branch))
        .await?
        .ok_or_else(|| GitError::new(GitErrorKind::RemoteGone, format!("origin/{branch} not found")))?;
    let head = repo::head_sha(git, repo).await?.unwrap_or_default();
    if head == target {
        // Still discard any tracked edits, which is what the user asked for.
        discard_changes(git, repo).await?;
        return Ok(TickResult { state: SyncState::Idle, head: Some(head.clone()), moved: None, synced_sha: Some(head) });
    }
    reset_hard(git, repo, &head, &target).await
}

#[derive(Debug, Clone, PartialEq)]
pub enum SwitchError {
    /// Tracked files are modified; needs "Discard and switch".
    Dirty(Vec<String>),
    /// Untracked files would be overwritten; needs "Move files aside".
    UntrackedInTheWay(Vec<String>),
    Git(GitError),
}

impl From<GitError> for SwitchError {
    fn from(e: GitError) -> Self {
        match e.kind {
            GitErrorKind::WouldOverwrite { paths, untracked: true } => SwitchError::UntrackedInTheWay(paths),
            GitErrorKind::WouldOverwrite { paths, untracked: false } => SwitchError::Dirty(paths),
            _ => SwitchError::Git(e),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchResult {
    pub tick: TickResult,
    pub from_branch: Option<String>,
    pub old_head: Option<String>,
    pub new_head: Option<String>,
    /// Files that differ between the old and new HEAD (for change flags).
    pub files: Vec<String>,
}

/// Switch the checkout to `ctx.branch` and bring it up to date.
pub async fn switch(ctx: SyncCtx<'_>, discard: bool) -> Result<SwitchResult, SwitchError> {
    let SyncCtx { git, repo, branch, .. } = ctx;
    if repo::operation_in_progress(git, repo).await? {
        return Err(SwitchError::Git(GitError::other(
            "A rebase or merge is in progress in this repo. Finish or abort it first.",
        )));
    }
    let dirty = repo::dirty_files(git, repo).await?;
    if !dirty.is_empty() {
        if discard {
            discard_changes(git, repo).await?;
        } else {
            return Err(SwitchError::Dirty(dirty));
        }
    }
    git.run(repo, &["fetch", "--no-tags", "origin", &branch_refspec(branch)], Mode::Network)
        .await
        .map_err(|e| match e.kind {
            GitErrorKind::RemoteGone => SwitchError::Git(GitError::new(
                GitErrorKind::RemoteGone,
                format!("{branch} no longer exists on origin"),
            )),
            _ => SwitchError::Git(e),
        })?;

    let from_branch = repo::head_branch(git, repo).await?;
    let old_head = repo::head_sha(git, repo).await?;

    if from_branch.as_deref() != Some(branch) {
        if repo::local_branch_exists(git, repo, branch).await? {
            git.run(repo, &["switch", "--quiet", branch], Mode::Write).await?;
        } else {
            let start = format!("origin/{branch}");
            let tracked = git
                .run(repo, &["switch", "--quiet", "-c", branch, "--track", &start], Mode::Write)
                .await;
            match tracked {
                Ok(_) => {}
                Err(e) if matches!(e.kind, GitErrorKind::Other) => {
                    // e.g. a narrowed fetch refspec: create without upstream.
                    git.run(repo, &["switch", "--quiet", "-c", branch, "--no-track", &start], Mode::Write)
                        .await?;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    let tick = match decide(ctx, false).await {
        Ok(t) => t,
        Err(e) => TickResult::state(error_state(e), None),
    };
    let new_head = repo::head_sha(git, repo).await?;
    let files = match (&old_head, &new_head) {
        (Some(a), Some(b)) if a != b => repo::changed_files(git, repo, a, b).await.unwrap_or_default(),
        _ => vec![],
    };
    Ok(SwitchResult { tick, from_branch, old_head, new_head, files })
}

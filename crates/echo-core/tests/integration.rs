//! Integration tests against real git repos: a bare `origin`, an "agent"
//! clone that commits / pushes / force-pushes, and the "local" clone that
//! Echo keeps in sync.

use echo_core::branches::{self, PollCache};
use echo_core::git::{Git, GitErrorKind};
use echo_core::store::ForcePushPolicy;
use echo_core::sync::{self, SwitchError, SyncCtx};
use echo_core::types::{BranchStatus, PauseReason, SyncState};
use echo_core::{Core, CoreEvent, EventSink};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    origin: PathBuf,
    agent: PathBuf,
    local: PathBuf,
    git: Git,
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=t@example.com", "-c", "init.defaultBranch=main"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

impl Fx {
    async fn new() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let origin = root.join("origin.git");
        let agent = root.join("agent");
        let local = root.join("local");
        sh(&root, &["init", "--bare", "origin.git"]);
        sh(&root, &["clone", "-q", "origin.git", "agent"]);
        std::fs::write(agent.join("README.md"), "hello\n").unwrap();
        std::fs::write(agent.join("package.json"), "{}\n").unwrap();
        sh(&agent, &["add", "."]);
        sh(&agent, &["commit", "-qm", "Initial commit"]);
        sh(&agent, &["push", "-q", "origin", "HEAD:main"]);
        sh(&root, &["clone", "-q", "origin.git", "local"]);
        for d in [&agent, &local] {
            sh(d, &["config", "user.name", "Test"]);
            sh(d, &["config", "user.email", "t@example.com"]);
        }
        let git = Git::discover(Some(Path::new(&which_git()))).await.unwrap();
        Fx { _dir: dir, root, origin, agent, local, git }
    }

    /// Agent: commit a file on `branch` (creating it from main if needed) and push.
    fn agent_commit(&self, branch: &str, file: &str, content: &str, msg: &str) -> String {
        let exists = Command::new("git")
            .current_dir(&self.agent)
            .args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])
            .status()
            .unwrap()
            .success();
        if exists {
            sh(&self.agent, &["switch", "-q", branch]);
        } else {
            sh(&self.agent, &["switch", "-q", "-c", branch, "main"]);
        }
        let p = self.agent.join(file);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
        sh(&self.agent, &["add", "-A"]);
        sh(&self.agent, &["commit", "-qm", msg]);
        sh(&self.agent, &["push", "-q", "-f", "origin", &format!("{branch}:{branch}")]);
        sh(&self.agent, &["rev-parse", "HEAD"])
    }

    /// Agent: rewrite the last commit on `branch` and force-push.
    fn agent_force_push(&self, branch: &str) -> String {
        sh(&self.agent, &["switch", "-q", branch]);
        std::fs::write(self.agent.join("rewritten.txt"), "rewritten\n").unwrap();
        sh(&self.agent, &["add", "-A"]);
        sh(&self.agent, &["commit", "-q", "--amend", "-m", "Rewritten"]);
        sh(&self.agent, &["push", "-q", "-f", "origin", &format!("{branch}:{branch}")]);
        sh(&self.agent, &["rev-parse", "HEAD"])
    }

    fn local_head(&self) -> String {
        sh(&self.local, &["rev-parse", "HEAD"])
    }

    fn ctx<'a>(&'a self, branch: &'a str, last_synced: Option<&'a str>) -> SyncCtx<'a> {
        SyncCtx::new(&self.git, &self.local, branch).last_synced(last_synced)
    }

    async fn switch(&self, branch: &str) -> sync::SwitchResult {
        sync::switch(self.ctx(branch, None), false).await.expect("switch")
    }
}

fn which_git() -> String {
    let out = Command::new("git").args(["--exec-path"]).output().unwrap();
    let exec = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let candidate = Path::new(&exec).join("git");
    if candidate.exists() {
        return candidate.display().to_string();
    }
    "git".into()
}

// ---------------------------------------------------------------------------
// Sync state machine
// ---------------------------------------------------------------------------

#[tokio::test]
async fn switch_to_new_branch_then_fast_forward() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    let r = fx.switch("claude/a").await;
    assert_eq!(r.tick.state, SyncState::Idle);
    assert_eq!(fx.local_head(), sha1);
    assert_eq!(r.from_branch.as_deref(), Some("main"));
    assert_eq!(r.files, vec!["a.txt"]);
    assert_eq!(sh(&fx.local, &["rev-parse", "--abbrev-ref", "@{u}"]), "origin/claude/a");

    // Idle tick: nothing to do.
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(t.state, SyncState::Idle);
    assert!(t.moved.is_none());

    // Agent pushes two commits: fast-forward.
    fx.agent_commit("claude/a", "b.txt", "2", "Second");
    let sha3 = fx.agent_commit("claude/a", "pnpm-lock.yaml", "x", "Third");
    let pulled = std::sync::atomic::AtomicBool::new(false);
    let on_pull = || pulled.store(true, std::sync::atomic::Ordering::SeqCst);
    let ctx = SyncCtx { on_pull: Some(&on_pull), ..fx.ctx("claude/a", Some(&sha1)) };
    let t = sync::tick(ctx).await;
    assert!(pulled.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(t.state, SyncState::Idle);
    let m = t.moved.unwrap();
    assert_eq!((m.count, m.reset, m.subject.as_str()), (2, false, "Third"));
    assert_eq!(m.files, vec!["b.txt", "pnpm-lock.yaml"]);
    assert_eq!(t.synced_sha.as_deref(), Some(sha3.as_str()));
    assert_eq!(fx.local_head(), sha3);
}

#[tokio::test]
async fn switch_back_to_existing_local_branch_fast_forwards() {
    let fx = Fx::new().await;
    fx.agent_commit("claude/a", "a.txt", "1", "A1");
    fx.agent_commit("claude/b", "b.txt", "1", "B1");
    fx.switch("claude/a").await;
    fx.switch("claude/b").await;
    let a2 = fx.agent_commit("claude/a", "a.txt", "2", "A2");
    let r = fx.switch("claude/a").await;
    assert_eq!(r.tick.state, SyncState::Idle);
    assert_eq!(fx.local_head(), a2);
    assert_eq!(r.from_branch.as_deref(), Some("claude/b"));
    assert!(r.files.contains(&"a.txt".to_string()) && r.files.contains(&"b.txt".to_string()));
}

#[tokio::test]
async fn force_push_with_clean_local_resets() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    let rewritten = fx.agent_force_push("claude/a");
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(t.state, SyncState::Idle);
    assert!(t.moved.as_ref().unwrap().reset);
    assert_eq!(fx.local_head(), rewritten);
}

#[tokio::test]
async fn force_push_with_ask_policy_blocks_until_confirmed() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    let rewritten = fx.agent_force_push("claude/a");
    let ctx = fx.ctx("claude/a", Some(&sha1)).policy(ForcePushPolicy::Ask);
    let t = sync::tick(ctx).await;
    assert_eq!(t.state, SyncState::BlockedForcePush { remote_sha: rewritten.clone() });
    assert_eq!(fx.local_head(), sha1);
    let t = sync::reset_to_remote(ctx).await.unwrap();
    assert_eq!(t.state, SyncState::Idle);
    assert_eq!(fx.local_head(), rewritten);
}

#[tokio::test]
async fn force_push_with_local_commits_is_diverged() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    std::fs::write(fx.local.join("mine.txt"), "mine").unwrap();
    sh(&fx.local, &["add", "mine.txt"]);
    sh(&fx.local, &["commit", "-qm", "Local work"]);
    fx.agent_force_push("claude/a");
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    match t.state {
        SyncState::BlockedDiverged { commits } => assert_eq!(commits[0].subject, "Local work"),
        s => panic!("expected diverged, got {s:?}"),
    }
}

#[tokio::test]
async fn local_commits_ahead_block() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    std::fs::write(fx.local.join("mine.txt"), "mine").unwrap();
    sh(&fx.local, &["add", "mine.txt"]);
    sh(&fx.local, &["commit", "-qm", "Local work"]);
    fx.agent_commit("claude/a", "a.txt", "2", "Second");
    // Remote moved, local has its own commit on top of an older remote: diverged.
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert!(matches!(t.state, SyncState::BlockedDiverged { .. }), "{:?}", t.state);

    // Pure "ahead": remote unchanged, local has a commit.
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/b", "a.txt", "1", "First");
    fx.switch("claude/b").await;
    std::fs::write(fx.local.join("mine.txt"), "mine").unwrap();
    sh(&fx.local, &["add", "mine.txt"]);
    sh(&fx.local, &["commit", "-qm", "Local work"]);
    let t = sync::tick(fx.ctx("claude/b", Some(&sha1))).await;
    match t.state {
        SyncState::BlockedLocalCommits { commits } => assert_eq!(commits.len(), 1),
        s => panic!("expected local commits, got {s:?}"),
    }
    let t = sync::reset_to_remote(fx.ctx("claude/b", Some(&sha1))).await.unwrap();
    assert_eq!(t.state, SyncState::Idle);
    assert_eq!(fx.local_head(), sha1);
}

#[tokio::test]
async fn dirty_tracked_file_blocks_and_discard_recovers() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    std::fs::write(fx.local.join("a.txt"), "local edit").unwrap();
    std::fs::write(fx.local.join("untracked.env"), "keep me").unwrap();
    let sha2 = fx.agent_commit("claude/a", "a.txt", "2", "Second");
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(
        t.state,
        SyncState::BlockedDirty { files: vec!["a.txt".into()], untracked: vec![], crlf_hint: false }
    );
    sync::discard_changes(&fx.git, &fx.local).await.unwrap();
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(t.state, SyncState::Idle);
    assert_eq!(fx.local_head(), sha2);
    assert!(fx.local.join("untracked.env").exists(), "untracked files survive discard");
}

#[tokio::test]
async fn untracked_file_in_the_way_blocks_and_move_aside_recovers() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    std::fs::create_dir_all(fx.local.join("cfg")).unwrap();
    std::fs::write(fx.local.join("cfg/new.json"), "mine").unwrap();
    fx.agent_commit("claude/a", "cfg/new.json", "theirs", "Add cfg");
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    match &t.state {
        SyncState::BlockedDirty { untracked, files, .. } => {
            assert_eq!(untracked, &vec!["cfg/new.json".to_string()]);
            assert!(files.is_empty());
        }
        s => panic!("expected blocked dirty, got {s:?}"),
    }
    let moved = sync::move_untracked_aside(&fx.git, &fx.local, &["cfg/new.json".into()], 42).await.unwrap();
    assert_eq!(moved, vec!["cfg/new.json.echo-42"]);
    assert_eq!(std::fs::read_to_string(fx.local.join("cfg/new.json.echo-42")).unwrap(), "mine");
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(t.state, SyncState::Idle);
    assert_eq!(std::fs::read_to_string(fx.local.join("cfg/new.json")).unwrap(), "theirs");
}

#[tokio::test]
async fn force_push_reset_never_deletes_untracked_files() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    std::fs::write(fx.local.join("rewritten.txt"), "precious").unwrap();
    fx.agent_force_push("claude/a"); // adds rewritten.txt
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert!(matches!(&t.state, SyncState::BlockedDirty { untracked, .. } if untracked == &vec!["rewritten.txt".to_string()]));
    assert_eq!(std::fs::read_to_string(fx.local.join("rewritten.txt")).unwrap(), "precious");
}

#[tokio::test]
async fn move_aside_refuses_tracked_and_escaping_paths() {
    let fx = Fx::new().await;
    assert!(sync::move_untracked_aside(&fx.git, &fx.local, &["README.md".into()], 1).await.is_err());
    assert!(sync::move_untracked_aside(&fx.git, &fx.local, &["../agent/README.md".into()], 1).await.is_err());
    assert!(fx.local.join("README.md").exists());
}

#[tokio::test]
async fn switch_with_dirty_tree_requires_discard() {
    let fx = Fx::new().await;
    fx.agent_commit("claude/a", "a.txt", "1", "First");
    std::fs::write(fx.local.join("README.md"), "edited").unwrap();
    match sync::switch(fx.ctx("claude/a", None), false).await {
        Err(SwitchError::Dirty(files)) => assert_eq!(files, vec!["README.md"]),
        r => panic!("expected dirty, got {r:?}"),
    }
    assert_eq!(sh(&fx.local, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    let r = sync::switch(fx.ctx("claude/a", None), true).await.unwrap();
    assert_eq!(r.tick.state, SyncState::Idle);
    assert_eq!(std::fs::read_to_string(fx.local.join("README.md")).unwrap(), "hello\n");
}

#[tokio::test]
async fn switch_with_untracked_in_the_way() {
    let fx = Fx::new().await;
    fx.agent_commit("claude/a", "a.txt", "1", "First");
    std::fs::write(fx.local.join("a.txt"), "mine").unwrap();
    match sync::switch(fx.ctx("claude/a", None), false).await {
        Err(SwitchError::UntrackedInTheWay(p)) => assert_eq!(p, vec!["a.txt"]),
        r => panic!("expected untracked, got {r:?}"),
    }
}

#[tokio::test]
async fn switch_to_missing_branch_errors() {
    let fx = Fx::new().await;
    match sync::switch(fx.ctx("claude/nope", None), false).await {
        Err(SwitchError::Git(e)) => assert_eq!(e.kind, GitErrorKind::RemoteGone),
        r => panic!("expected remote gone, got {r:?}"),
    }
}

#[tokio::test]
async fn remote_branch_deleted() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    sh(&fx.agent, &["push", "-q", "origin", "--delete", "claude/a"]);
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(t.state, SyncState::RemoteBranchDeleted);
}

#[tokio::test]
async fn checkout_changed_outside_app_pauses_and_recovers() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    sh(&fx.local, &["switch", "-q", "main"]);
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(
        t.state,
        SyncState::Paused { reason: PauseReason::CheckoutChanged { current: Some("main".into()) } }
    );
    sh(&fx.local, &["switch", "-q", "claude/a"]);
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(t.state, SyncState::Idle);
}

#[tokio::test]
async fn rebase_in_progress_pauses() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    let git_dir = fx.local.join(".git");
    std::fs::create_dir_all(git_dir.join("rebase-merge")).unwrap();
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(t.state, SyncState::Paused { reason: PauseReason::GitOperation });
    assert!(matches!(sync::switch(fx.ctx("main", None), false).await, Err(SwitchError::Git(_))));
    std::fs::remove_dir_all(git_dir.join("rebase-merge")).unwrap();
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    assert_eq!(t.state, SyncState::Idle);
}

#[tokio::test]
async fn lock_file_retries_then_succeeds() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    let sha2 = fx.agent_commit("claude/a", "a.txt", "2", "Second");
    let lock = fx.local.join(".git/index.lock");
    std::fs::write(&lock, "").unwrap();
    let lock2 = lock.clone();
    let remover = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        std::fs::remove_file(lock2).unwrap();
    });
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    remover.await.unwrap();
    assert_eq!(t.state, SyncState::Idle, "should succeed after the lock is released");
    assert_eq!(fx.local_head(), sha2);
}

#[tokio::test]
async fn lock_file_held_surfaces_error() {
    let fx = Fx::new().await;
    let sha1 = fx.agent_commit("claude/a", "a.txt", "1", "First");
    fx.switch("claude/a").await;
    fx.agent_commit("claude/a", "a.txt", "2", "Second");
    std::fs::write(fx.local.join(".git/index.lock"), "").unwrap();
    let t = sync::tick(fx.ctx("claude/a", Some(&sha1))).await;
    match t.state {
        SyncState::Error { error } => assert_eq!(error.kind, GitErrorKind::LockHeld),
        s => panic!("expected lock error, got {s:?}"),
    }
}

// ---------------------------------------------------------------------------
// Branch poll
// ---------------------------------------------------------------------------

#[tokio::test]
async fn poll_reports_new_unseen_rewritten_and_deleted() {
    let fx = Fx::new().await;
    let a1 = fx.agent_commit("claude/a", "a.txt", "1", "A1");
    let b1 = fx.agent_commit("claude/b", "b.txt", "1", "B1");
    let main = sh(&fx.agent, &["rev-parse", "main"]);
    let mut cache = PollCache::default();
    let mut seen = BTreeMap::new();
    seen.insert("main".to_string(), main);
    seen.insert("claude/a".to_string(), a1.clone());

    let out = branches::poll(&fx.git, &fx.local, &mut cache, &seen, None).await.unwrap();
    assert!(out.changed);
    let status = |out: &branches::PollOutcome, n: &str| out.branches.iter().find(|b| b.name == n).unwrap().status.clone();
    assert_eq!(status(&out, "main"), BranchStatus::Seen);
    assert_eq!(status(&out, "claude/a"), BranchStatus::Seen);
    assert_eq!(status(&out, "claude/b"), BranchStatus::New);
    assert_eq!(out.current_branch.as_deref(), Some("main"));
    let b = out.branches.iter().find(|b| b.name == "claude/b").unwrap();
    assert_eq!((b.subject.as_str(), b.author.as_str(), b.sha.as_str()), ("B1", "Test", b1.as_str()));
    assert!(out.branches.iter().all(|b| b.name != "HEAD"));

    // No change on the remote: no fetch.
    let out = branches::poll(&fx.git, &fx.local, &mut cache, &seen, None).await.unwrap();
    assert!(!out.changed);

    // Two commits on a: unseen 2. Rewrite b after marking it seen: rewritten. Delete-later branch c.
    fx.agent_commit("claude/a", "a.txt", "2", "A2");
    fx.agent_commit("claude/a", "a.txt", "3", "A3");
    seen.insert("claude/b".into(), b1);
    fx.agent_force_push("claude/b");
    fx.agent_commit("claude/c", "c.txt", "1", "C1");
    let out = branches::poll(&fx.git, &fx.local, &mut cache, &seen, None).await.unwrap();
    assert!(out.changed);
    assert_eq!(out.new_branches, vec!["claude/c"]);
    assert_eq!(status(&out, "claude/a"), BranchStatus::Unseen { count: 2 });
    assert_eq!(status(&out, "claude/b"), BranchStatus::Rewritten);
    assert!(out.branches.windows(2).all(|w| w[0].committed_at >= w[1].committed_at), "sorted newest first");

    sh(&fx.agent, &["push", "-q", "origin", "--delete", "claude/c"]);
    let out = branches::poll(&fx.git, &fx.local, &mut cache, &seen, None).await.unwrap();
    assert_eq!(out.deleted_branches, vec!["claude/c"]);
    let c = out.branches.iter().find(|b| b.name == "claude/c").expect("deleted shown once");
    assert!(c.deleted);
    let out = branches::poll(&fx.git, &fx.local, &mut cache, &seen, None).await.unwrap();
    assert!(out.branches.iter().all(|b| b.name != "claude/c"), "then dropped");
    assert!(sh(&fx.local, &["branch", "-r"]).lines().all(|l| !l.contains("claude/c")), "pruned");
}

#[tokio::test]
async fn poll_against_unreachable_origin_is_classified() {
    let fx = Fx::new().await;
    sh(&fx.local, &["remote", "set-url", "origin", &fx.root.join("missing.git").display().to_string()]);
    let mut cache = PollCache::default();
    let err = branches::poll(&fx.git, &fx.local, &mut cache, &BTreeMap::new(), None).await.unwrap_err();
    assert_ne!(err.kind, GitErrorKind::LockHeld);
    assert!(!err.message.is_empty());
    let _ = &fx.origin;
}

// ---------------------------------------------------------------------------
// Core end to end
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Recorder(Mutex<Vec<CoreEvent>>);
impl EventSink for Recorder {
    fn emit(&self, event: CoreEvent) {
        self.0.lock().unwrap().push(event);
    }
}
impl Recorder {
    fn notes(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                CoreEvent::Notify { body, .. } => Some(body.clone()),
                _ => None,
            })
            .collect()
    }
}

async fn wait_for(label: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..100 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("condition not met in time: {label}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_end_to_end() {
    let fx = Fx::new().await;
    fx.agent_commit("claude/old", "o.txt", "1", "Old work");
    let data = fx.root.join("appdata");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("config.json"),
        format!(r#"{{"gitPath": {:?}, "syncBaseSeconds": 1}}"#, which_git()),
    )
    .unwrap();
    let rec = Arc::new(Recorder::default());
    let core = Core::start(&data, rec.clone(), "test").await;
    assert!(core.app_info().git_error.is_none());

    // Adding seeds the seen-state: existing branches don't light up.
    let summary = core.add_repo(fx.local.join("src-does-not-exist").to_str().unwrap()).await;
    assert!(summary.is_err());
    let summary = core.add_repo(fx.local.to_str().unwrap()).await.unwrap();
    let id = summary.id.clone();
    assert!(!summary.has_unseen);
    assert!(core.add_repo(fx.local.to_str().unwrap()).await.is_err(), "no duplicates");
    let snap = core.get_branches(&id).unwrap();
    assert!(snap.branches.iter().all(|b| b.status == BranchStatus::Seen));

    // A new agent branch shows as new after a poll.
    fx.agent_commit("claude/feat", "f.txt", "1", "Feature");
    core.sync_now(&id).await.unwrap();
    wait_for("w1", || {
        core.get_branches(&id).unwrap().branches.iter().any(|b| b.name == "claude/feat" && b.status == BranchStatus::New)
    })
    .await;
    assert!(core.list_repos()[0].has_unseen);
    core.mark_seen(&id, Some("claude/feat")).unwrap();
    assert!(!core.list_repos()[0].has_unseen);

    // Start syncing; agent pushes a dependency change; the core pulls it.
    let st = core.start_sync(&id, "claude/feat", false).await.unwrap();
    assert_eq!(st.state, SyncState::Idle);
    let sha = fx.agent_commit("claude/feat", "package.json", "{\"x\":1}", "Bump deps");
    wait_for("w2", || fx.local_head() == sha).await;
    wait_for("w3", || core.list_repos()[0].flags.iter().any(|f| f.kind == echo_core::flags::FlagKind::Deps)).await;
    wait_for("w4", || rec.notes().iter().any(|n| n.contains("claude/feat → 1 commit: Bump deps") && n.contains("Dependencies changed")))
        .await;
    // Synced branch is marked seen when pulled.
    wait_for("w5", || {
        core.get_branches(&id).unwrap().branches.iter().any(|b| b.name == "claude/feat" && b.status == BranchStatus::Seen && b.sha == sha)
    })
    .await;

    // Dirty file blocks; discard recovers.
    std::fs::write(fx.local.join("f.txt"), "local edit").unwrap();
    let sha2 = fx.agent_commit("claude/feat", "f.txt", "2", "Edit f");
    wait_for("w6", || matches!(core.get_sync_status(&id).unwrap().state, SyncState::BlockedDirty { .. })).await;
    let st = core.discard_changes(&id).await.unwrap();
    assert_eq!(st.state, SyncState::Idle);
    assert_eq!(fx.local_head(), sha2);

    // Pause / resume / stop.
    assert!(matches!(core.pause_sync(&id).unwrap().state, SyncState::Paused { .. }));
    assert_eq!(core.resume_sync(&id).await.unwrap().state, SyncState::Idle);
    core.dismiss_flag(&id, echo_core::flags::FlagKind::Deps).unwrap();
    assert!(core.list_repos()[0].flags.is_empty());

    // Settings patch validation.
    assert!(core.update_settings(serde_json::json!({"repos": []})).await.is_err());
    let s = core.update_settings(serde_json::json!({"branchPollSeconds": 1})).await.unwrap();
    assert_eq!(s.branch_poll_seconds, 5, "clamped");
    let r = core.update_repo_config(&id, serde_json::json!({"displayName": "renamed"})).unwrap();
    assert_eq!(r.config.display_name, "renamed");

    assert_eq!(core.stop_sync(&id).unwrap().state, SyncState::Off);
    core.shutdown();

    // Everything persisted.
    let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(data.join("config.json")).unwrap()).unwrap();
    assert_eq!(config["repos"][0]["displayName"], "renamed");
    let state: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(data.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["repos"][&id]["lastSeen"]["claude/feat"], sha2);
    assert_eq!(state["repos"][&id]["lastSynced"]["claude/feat"], sha2);

    let tray = core.tray_model();
    assert!(tray.synced.is_empty());
    core.remove_repo(&id).unwrap();
    assert!(core.list_repos().is_empty());
}

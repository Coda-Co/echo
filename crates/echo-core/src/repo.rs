//! Repo-level git helpers shared by the poll and sync logic.

use crate::git::{parse_status_z, Git, GitError, GitErrorKind, Mode};
use crate::types::CommitSummary;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ValidatedRepo {
    pub toplevel: PathBuf,
    pub origin_url: String,
    pub current_branch: Option<String>,
}

/// Validate a folder picked by the user (see "Validate a repo" in the plan).
pub async fn validate(git: &Git, picked: &Path) -> Result<ValidatedRepo, GitError> {
    if !picked.is_dir() {
        return Err(GitError::new(GitErrorKind::NotARepo, format!("{} is not a folder", picked.display())));
    }
    let top = git.run(picked, &["rev-parse", "--show-toplevel"], Mode::Read).await.map_err(|e| match e.kind {
        GitErrorKind::NotARepo => {
            GitError::new(GitErrorKind::NotARepo, format!("{} is not inside a git repository", picked.display()))
        }
        _ => e,
    })?;
    let toplevel = PathBuf::from(top.trim());
    let toplevel = dunce_canonical(&toplevel);
    let origin_url = git
        .run(&toplevel, &["remote", "get-url", "origin"], Mode::Read)
        .await
        .map_err(|_| GitError::other("This repository has no remote called \"origin\". Echo only syncs from origin."))?
        .trim()
        .to_string();
    let current_branch = head_branch(git, &toplevel).await?;
    Ok(ValidatedRepo { toplevel, origin_url, current_branch })
}

/// Canonicalise without Windows `\\?\` prefixes.
fn dunce_canonical(p: &Path) -> PathBuf {
    let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    #[cfg(windows)]
    {
        let s = c.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            if !stripped.starts_with("UNC\\") {
                return PathBuf::from(stripped);
            }
        }
    }
    c
}

/// Current branch name, or `None` when HEAD is detached.
pub async fn head_branch(git: &Git, repo: &Path) -> Result<Option<String>, GitError> {
    let out = git.exec(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"], Mode::Read).await?;
    match out.code {
        Some(0) => Ok(Some(out.stdout.trim().to_string())),
        Some(1) => Ok(None),
        _ => Err(crate::git::classify(&out.stderr)),
    }
}

/// HEAD sha, or `None` in an empty repo.
pub async fn head_sha(git: &Git, repo: &Path) -> Result<Option<String>, GitError> {
    rev_parse(git, repo, "HEAD").await
}

pub async fn rev_parse(git: &Git, repo: &Path, rev: &str) -> Result<Option<String>, GitError> {
    let spec = format!("{rev}^{{commit}}");
    let out = git.exec(repo, &["rev-parse", "--verify", "--quiet", &spec], Mode::Read).await?;
    if out.success() {
        Ok(Some(out.stdout.trim().to_string()))
    } else {
        Ok(None)
    }
}

/// Modified tracked files (untracked files are ignored).
pub async fn dirty_files(git: &Git, repo: &Path) -> Result<Vec<String>, GitError> {
    let out = git.run(repo, &["status", "--porcelain=v1", "-z", "--untracked-files=no"], Mode::Read).await?;
    Ok(parse_status_z(&out))
}

/// True when every modified tracked file differs only in line endings.
pub async fn dirty_only_line_endings(git: &Git, repo: &Path, files: &[String]) -> bool {
    if files.is_empty() {
        return false;
    }
    match git.exec(repo, &["diff", "--ignore-cr-at-eol", "--quiet"], Mode::Read).await {
        Ok(out) => out.code == Some(0),
        Err(_) => false,
    }
}

/// Absolute git dir.
pub async fn git_dir(git: &Git, repo: &Path) -> Result<PathBuf, GitError> {
    let out = git.run(repo, &["rev-parse", "--absolute-git-dir"], Mode::Read).await?;
    Ok(PathBuf::from(out.trim()))
}

/// A rebase, merge, cherry-pick or revert is in progress.
pub async fn operation_in_progress(git: &Git, repo: &Path) -> Result<bool, GitError> {
    let dir = git_dir(git, repo).await?;
    Ok(["rebase-merge", "rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"]
        .iter()
        .any(|p| dir.join(p).exists()))
}

pub async fn is_ancestor(git: &Git, repo: &Path, a: &str, b: &str) -> Result<bool, GitError> {
    git.check(repo, &["merge-base", "--is-ancestor", a, b]).await
}

pub async fn local_branch_exists(git: &Git, repo: &Path, branch: &str) -> Result<bool, GitError> {
    let r = format!("refs/heads/{branch}");
    git.check(repo, &["show-ref", "--verify", "--quiet", &r]).await
}

pub const LOG_FORMAT: &str = "--format=%H%x00%s%x00%an%x00%ct";

pub fn parse_log(out: &str) -> Vec<CommitSummary> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\0');
            Some(CommitSummary {
                sha: f.next()?.to_string(),
                subject: f.next().unwrap_or("").to_string(),
                author: f.next().unwrap_or("").to_string(),
                committed_at: f.next().and_then(|t| t.parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

pub async fn log(git: &Git, repo: &Path, range: &str, n: usize) -> Result<Vec<CommitSummary>, GitError> {
    let n = format!("-n{n}");
    let out = git.run(repo, &["log", LOG_FORMAT, &n, range, "--"], Mode::Read).await?;
    Ok(parse_log(&out))
}

pub async fn count(git: &Git, repo: &Path, range: &str) -> Result<u32, GitError> {
    let out = git.run(repo, &["rev-list", "--count", range], Mode::Read).await?;
    Ok(out.trim().parse().unwrap_or(0))
}

pub async fn changed_files(git: &Git, repo: &Path, from: &str, to: &str) -> Result<Vec<String>, GitError> {
    let out = git.run(repo, &["diff", "--name-only", "--no-renames", "-z", from, to], Mode::Read).await?;
    Ok(out.split('\0').filter(|s| !s.is_empty()).map(String::from).collect())
}

/// Untracked files on disk that a move from HEAD to `target` would overwrite.
/// `git reset --hard` deletes these silently, so we check before resetting.
pub async fn untracked_in_the_way(git: &Git, repo: &Path, target: &str) -> Result<Vec<String>, GitError> {
    let base = head_sha(git, repo).await?;
    let added: Vec<String> = match base {
        Some(head) => {
            let out = git
                .run(repo, &["diff", "--name-only", "--no-renames", "--diff-filter=A", "-z", &head, target], Mode::Read)
                .await?;
            out.split('\0').filter(|s| !s.is_empty()).map(String::from).collect()
        }
        None => {
            let out = git.run(repo, &["ls-tree", "-r", "-z", "--name-only", target], Mode::Read).await?;
            out.split('\0').filter(|s| !s.is_empty()).map(String::from).collect()
        }
    };
    Ok(added.into_iter().filter(|p| repo.join(p).symlink_metadata().is_ok()).collect())
}

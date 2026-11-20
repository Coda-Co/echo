//! Branch poll: detect remote changes cheaply with `ls-remote`, fetch only when
//! something moved, then compute per-branch activity against the seen-state.

use crate::git::{parse_for_each_ref, parse_ls_remote, Git, GitError, Mode, FOR_EACH_REF_FORMAT};
use crate::repo;
use crate::types::{BranchInfo, BranchStatus};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub const FETCH_ALL_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";

/// In-memory state carried between polls (not persisted).
#[derive(Debug, Default, Clone)]
pub struct PollCache {
    /// Last `ls-remote` result.
    pub remote: Option<BTreeMap<String, String>>,
    /// branch -> (tip, last_seen, status) memo, so unchanged branches skip rev-list.
    statuses: HashMap<String, (String, Option<String>, BranchStatus)>,
    /// Branches returned by the previous poll (for "deleted" display).
    prev: Vec<BranchInfo>,
    /// Display data from the last for-each-ref.
    refs: Vec<crate::git::RefInfo>,
}

#[derive(Debug, Clone)]
pub struct PollOutcome {
    pub branches: Vec<BranchInfo>,
    pub current_branch: Option<String>,
    /// The remote changed since the previous poll (and was fetched).
    pub changed: bool,
    /// Branches that appeared since the previous poll (empty on first poll).
    pub new_branches: Vec<String>,
    /// Branches that disappeared since the previous poll.
    pub deleted_branches: Vec<String>,
}

impl PollCache {
    /// Force the next poll to fetch (e.g. after the user asks to refresh).
    pub fn invalidate(&mut self) {
        self.remote = None;
    }

    /// Current tip of a branch according to the last poll.
    pub fn tip(&self, branch: &str) -> Option<&str> {
        self.remote.as_ref()?.get(branch).map(String::as_str)
    }

    /// Recompute statuses from cached refs after the seen-state changed (no network).
    pub async fn restatus(
        &mut self,
        git: &Git,
        repo: &Path,
        last_seen: &BTreeMap<String, String>,
        current: Option<&str>,
        synced: Option<&str>,
    ) -> Vec<BranchInfo> {
        let refs = self.refs.clone();
        let mut out = Vec::with_capacity(refs.len());
        for r in refs {
            let status = self.status_for(git, repo, &r.name, &r.sha, last_seen.get(&r.name)).await;
            out.push(make_info(r, status, current, synced));
        }
        sort(&mut out);
        self.prev = out.clone();
        out
    }

    async fn status_for(
        &mut self,
        git: &Git,
        repo: &Path,
        name: &str,
        tip: &str,
        seen: Option<&String>,
    ) -> BranchStatus {
        if let Some((t, s, st)) = self.statuses.get(name) {
            if t == tip && s.as_ref() == seen {
                return st.clone();
            }
        }
        let st = compute_status(git, repo, tip, seen.map(String::as_str)).await;
        self.statuses.insert(name.to_string(), (tip.to_string(), seen.cloned(), st.clone()));
        st
    }
}

/// Status of a branch tip relative to its last-seen sha.
pub async fn compute_status(git: &Git, repo: &Path, tip: &str, seen: Option<&str>) -> BranchStatus {
    let Some(seen) = seen else { return BranchStatus::New };
    if seen == tip {
        return BranchStatus::Seen;
    }
    let range = format!("{seen}...{tip}");
    match git.run(repo, &["rev-list", "--left-right", "--count", &range], Mode::Read).await {
        Ok(out) => {
            let mut parts = out.split_whitespace().map(|p| p.parse::<u32>().unwrap_or(0));
            let left = parts.next().unwrap_or(0);
            let right = parts.next().unwrap_or(0);
            if left == 0 {
                BranchStatus::Unseen { count: right }
            } else {
                BranchStatus::Rewritten
            }
        }
        // Seen sha no longer exists locally: treat as rewritten.
        Err(_) => BranchStatus::Rewritten,
    }
}

fn make_info(r: crate::git::RefInfo, status: BranchStatus, current: Option<&str>, synced: Option<&str>) -> BranchInfo {
    BranchInfo {
        is_current: current == Some(r.name.as_str()),
        is_synced: synced == Some(r.name.as_str()),
        name: r.name,
        sha: r.sha,
        subject: r.subject,
        author: r.author,
        committed_at: r.committed_at,
        status,
        deleted: false,
    }
}

fn sort(v: &mut [BranchInfo]) {
    v.sort_by(|a, b| b.committed_at.cmp(&a.committed_at).then_with(|| a.name.cmp(&b.name)));
}

/// One branch-poll cycle.
pub async fn poll(
    git: &Git,
    repo: &Path,
    cache: &mut PollCache,
    last_seen: &BTreeMap<String, String>,
    synced: Option<&str>,
) -> Result<PollOutcome, GitError> {
    let out = git.run(repo, &["ls-remote", "--heads", "origin"], Mode::Network).await?;
    let remote: BTreeMap<String, String> = parse_ls_remote(&out).into_iter().collect();
    let changed = cache.remote.as_ref() != Some(&remote);

    let (new_branches, deleted_branches) = match &cache.remote {
        Some(prev) => (
            remote.keys().filter(|k| !prev.contains_key(*k)).cloned().collect(),
            prev.keys().filter(|k| !remote.contains_key(*k)).cloned().collect(),
        ),
        None => (Vec::new(), Vec::new()),
    };

    if changed {
        git.run(repo, &["fetch", "--prune", "--no-tags", "origin", FETCH_ALL_REFSPEC], Mode::Network).await?;
        let refs_out = git.run(repo, &["for-each-ref", "refs/remotes/origin", FOR_EACH_REF_FORMAT], Mode::Read).await?;
        cache.refs = parse_for_each_ref(&refs_out).into_iter().filter(|r| remote.contains_key(&r.name)).collect();
        cache.statuses.retain(|k, _| remote.contains_key(k));
        cache.remote = Some(remote);
    }

    let current = repo::head_branch(git, repo).await?;
    let prev_names: HashSet<String> = cache.prev.iter().filter(|b| !b.deleted).map(|b| b.name.clone()).collect();
    let prev_snapshot = std::mem::take(&mut cache.prev);
    let mut branches = cache.restatus(git, repo, last_seen, current.as_deref(), synced).await;

    // Show branches deleted in this cycle greyed out, once.
    let now: HashSet<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    let mut gone: Vec<BranchInfo> = prev_snapshot
        .into_iter()
        .filter(|b| !b.deleted && prev_names.contains(&b.name) && !now.contains(b.name.as_str()))
        .map(|mut b| {
            b.deleted = true;
            b.status = BranchStatus::Seen;
            b
        })
        .collect();
    branches.append(&mut gone);
    cache.prev = branches.clone();

    Ok(PollOutcome { branches, current_branch: current, changed, new_branches, deleted_branches })
}

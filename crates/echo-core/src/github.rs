//! GitHub integration: map `origin` to a GitHub repo, and list open pull
//! requests through the GitHub CLI (`gh`), which owns authentication. Echo
//! never reads or stores a GitHub token itself.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const GH_TIMEOUT: Duration = Duration::from_secs(30);
/// Enough for any repo Echo is realistically pointed at; older PRs are dropped.
const PR_LIMIT: &str = "200";

/// A repository on github.com.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepo {
    pub owner: String,
    pub name: String,
}

impl GitHubRepo {
    pub fn web_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }

    fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Parse a git remote URL pointing at github.com. Handles
/// `https://github.com/o/r(.git)`, `git@github.com:o/r(.git)` and
/// `ssh://git@github.com(:22)/o/r(.git)`. Anything else (other hosts,
/// GitHub Enterprise, local paths) returns `None`.
pub fn parse_github_remote(url: &str) -> Option<GitHubRepo> {
    let url = url.trim();
    let path = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .or_else(|| url.strip_prefix("ssh://"))
            .or_else(|| url.strip_prefix("git://"))?;
        // Drop userinfo (`git@`, `user:token@`) and an optional port.
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        let (host, path) = rest.split_once('/')?;
        let host = host.split(':').next()?;
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com") {
            return None;
        }
        path
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if parts.next().is_some() || !valid(owner) || !valid(name) || name == "." || name == ".." {
        return None;
    }
    Some(GitHubRepo { owner: owner.to_string(), name: name.to_string() })
}

/// An open pull request whose head is a branch of this repo.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub draft: bool,
}

/// Open PRs for a repo, keyed by head branch name, plus why they're missing.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestSnapshot {
    pub by_branch: BTreeMap<String, PullRequest>,
    /// Set when PRs couldn't be listed (gh missing, not logged in, offline…).
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPr {
    number: u64,
    title: String,
    url: String,
    is_draft: bool,
    head_ref_name: String,
    head_repository_owner: Option<GhOwner>,
}

#[derive(Deserialize)]
struct GhOwner {
    login: String,
}

/// Parse `gh pr list --json number,title,url,isDraft,headRefName,headRepositoryOwner`.
/// PRs from forks are dropped (their branch names aren't this repo's branches);
/// when a branch has several open PRs, the newest wins.
pub fn parse_pr_list(json: &str, repo: &GitHubRepo) -> Result<BTreeMap<String, PullRequest>, String> {
    let prs: Vec<GhPr> = serde_json::from_str(json).map_err(|e| format!("unexpected output from gh: {e}"))?;
    let prefix = format!("{}/", repo.web_url());
    let mut out: BTreeMap<String, PullRequest> = BTreeMap::new();
    for p in prs {
        let same_repo = p.head_repository_owner.as_ref().is_some_and(|o| o.login.eq_ignore_ascii_case(&repo.owner));
        // Only ever hand out links into this repo on github.com.
        if !same_repo || !p.url.starts_with(&prefix) {
            continue;
        }
        if out.get(&p.head_ref_name).is_some_and(|existing| existing.number > p.number) {
            continue;
        }
        out.insert(p.head_ref_name, PullRequest { number: p.number, title: p.title, url: p.url, draft: p.is_draft });
    }
    Ok(out)
}

/// The GitHub CLI.
#[derive(Debug, Clone)]
pub struct Gh {
    exe: PathBuf,
}

impl Gh {
    /// Find `gh` on PATH or in its usual install locations. Cheap (no process
    /// is started), so callers can re-run it to pick up a fresh install.
    pub fn discover() -> Option<Gh> {
        candidates().into_iter().find(|p| p.is_file()).map(|exe| Gh { exe })
    }

    pub fn path(&self) -> &Path {
        &self.exe
    }

    /// List open PRs for `repo`.
    pub async fn open_pull_requests(&self, repo: &GitHubRepo) -> Result<BTreeMap<String, PullRequest>, String> {
        let slug = repo.slug();
        let mut cmd = Command::new(&self.exe);
        cmd.args(["pr", "list", "--repo", &slug, "--state", "open", "--limit", PR_LIMIT])
            .args(["--json", "number,title,url,isDraft,headRefName,headRepositoryOwner"])
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        crate::git::no_window(&mut cmd);
        let child = cmd.spawn().map_err(|e| format!("could not run gh: {e}"))?;
        let out = tokio::time::timeout(GH_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| "gh timed out".to_string())?
            .map_err(|e| format!("gh failed: {e}"))?;
        if !out.status.success() {
            return Err(describe_gh_error(&String::from_utf8_lossy(&out.stderr)));
        }
        parse_pr_list(&String::from_utf8_lossy(&out.stdout), repo)
    }
}

fn describe_gh_error(stderr: &str) -> String {
    let s = stderr.to_ascii_lowercase();
    if s.contains("gh auth login") || s.contains("not logged in") || s.contains("authentication") {
        "GitHub CLI isn't logged in. Run `gh auth login`.".into()
    } else if s.contains("could not resolve") || s.contains("not found") {
        "GitHub CLI can't see this repo with your account.".into()
    } else {
        let first = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        format!("gh: {}", first.trim())
    }
}

fn candidates() -> Vec<PathBuf> {
    let exe_name = if cfg!(windows) { "gh.exe" } else { "gh" };
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        // GUI apps don't inherit the shell PATH.
        for p in ["/opt/homebrew/bin/gh", "/usr/local/bin/gh"] {
            out.push(PathBuf::from(p));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            out.push(dir.join(exe_name));
        }
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(var) {
                out.push(PathBuf::from(base).join("GitHub CLI").join("gh.exe"));
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(local).join("Programs").join("GitHub CLI").join("gh.exe"));
        }
    } else {
        out.push(PathBuf::from("/usr/bin/gh"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(owner: &str, name: &str) -> Option<GitHubRepo> {
        Some(GitHubRepo { owner: owner.into(), name: name.into() })
    }

    #[test]
    fn parses_github_remotes() {
        assert_eq!(parse_github_remote("https://github.com/Coda-Co/echo.git"), repo("Coda-Co", "echo"));
        assert_eq!(parse_github_remote("https://github.com/Coda-Co/echo"), repo("Coda-Co", "echo"));
        assert_eq!(parse_github_remote("https://github.com/Coda-Co/echo/"), repo("Coda-Co", "echo"));
        assert_eq!(parse_github_remote("https://user:tok@github.com/o/r.git"), repo("o", "r"));
        assert_eq!(parse_github_remote("git@github.com:Coda-Co/my.repo.git\n"), repo("Coda-Co", "my.repo"));
        assert_eq!(parse_github_remote("ssh://git@github.com/o/r.git"), repo("o", "r"));
        assert_eq!(parse_github_remote("ssh://git@github.com:22/o/r"), repo("o", "r"));
    }

    #[test]
    fn rejects_non_github_remotes() {
        for url in [
            "https://gitlab.com/o/r.git",
            "git@gitlab.com:o/r.git",
            "https://github.example.com/o/r.git",
            "https://github.com.evil.io/o/r",
            "/srv/git/repo.git",
            "C:\\repos\\origin.git",
            "https://github.com/o",
            "https://github.com/o/r/extra",
            "https://github.com/o/r%20x",
        ] {
            assert_eq!(parse_github_remote(url), None, "{url}");
        }
    }

    #[test]
    fn parses_pr_list_dropping_forks_and_keeping_newest() {
        let r = GitHubRepo { owner: "Coda-Co".into(), name: "echo".into() };
        let json = r#"[
          {"number": 12, "title": "Add filters", "url": "https://github.com/Coda-Co/echo/pull/12", "isDraft": false,
           "headRefName": "claude/filters", "headRepositoryOwner": {"login": "Coda-Co"}},
          {"number": 15, "title": "Filters v2", "url": "https://github.com/Coda-Co/echo/pull/15", "isDraft": true,
           "headRefName": "claude/filters", "headRepositoryOwner": {"login": "coda-co"}},
          {"number": 9, "title": "From a fork", "url": "https://github.com/Coda-Co/echo/pull/9", "isDraft": false,
           "headRefName": "main", "headRepositoryOwner": {"login": "someone"}},
          {"number": 3, "title": "Odd URL", "url": "https://example.com/pull/3", "isDraft": false,
           "headRefName": "x", "headRepositoryOwner": {"login": "Coda-Co"}}
        ]"#;
        let prs = parse_pr_list(json, &r).unwrap();
        assert_eq!(prs.len(), 1);
        let p = &prs["claude/filters"];
        assert_eq!((p.number, p.draft, p.title.as_str()), (15, true, "Filters v2"));
    }

    #[test]
    fn classifies_gh_errors() {
        assert!(
            describe_gh_error("To get started with GitHub CLI, please run:  gh auth login").contains("gh auth login")
        );
        assert_eq!(describe_gh_error("\nboom\nmore"), "gh: boom");
    }
}

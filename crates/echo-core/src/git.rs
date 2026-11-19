//! Thin wrapper around the system `git` CLI: discovery, invocation rules,
//! output parsers and stderr classification.
//!
//! Every git command Echo runs goes through [`Git::exec`], which applies the
//! invocation rules from the build plan: `-C <repo>`, `GIT_TERMINAL_PROMPT=0`,
//! `LC_ALL=C`, `GIT_OPTIONAL_LOCKS=0` for read-only commands, a timeout with
//! kill-on-timeout, and `CREATE_NO_WINDOW` on Windows.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Minimum git version (for `git switch`).
pub const MIN_GIT_VERSION: (u32, u32, u32) = (2, 23, 0);

const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
const LOCAL_TIMEOUT: Duration = Duration::from_secs(120);

/// Typed classification of a git failure, for the UI.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GitErrorKind {
    LockHeld,
    Auth,
    Network,
    NotFastForward,
    #[serde(rename_all = "camelCase")]
    WouldOverwrite {
        paths: Vec<String>,
        untracked: bool,
    },
    RemoteGone,
    NotARepo,
    GitNotFound,
    Other,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct GitError {
    pub kind: GitErrorKind,
    pub message: String,
}

impl GitError {
    pub fn new(kind: GitErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
    pub fn other(message: impl Into<String>) -> Self {
        Self::new(GitErrorKind::Other, message)
    }
    pub fn is(&self, kind: &GitErrorKind) -> bool {
        std::mem::discriminant(&self.kind) == std::mem::discriminant(kind)
    }
}

/// How a command touches the repo / network. Drives env vars and timeouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Local, read-only (`GIT_OPTIONAL_LOCKS=0`).
    Read,
    /// Local, may write the index / refs / worktree. Retries on lock contention.
    Write,
    /// Talks to the remote (30 s timeout). May write refs (fetch).
    Network,
}

#[derive(Debug, Clone)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

#[derive(Debug, Clone)]
pub struct Git {
    exe: PathBuf,
    version: (u32, u32, u32),
}

impl Git {
    /// Locate git (user override first), check its version and return a runner.
    pub async fn discover(override_path: Option<&Path>) -> Result<Git, GitError> {
        let candidates: Vec<PathBuf> = match override_path {
            Some(p) => vec![p.to_path_buf()],
            None => default_candidates(),
        };
        let mut last_err = None;
        for exe in candidates {
            if !exe.is_file() {
                continue;
            }
            match probe_version(&exe).await {
                Ok(version) if version >= MIN_GIT_VERSION => return Ok(Git { exe, version }),
                Ok(version) => {
                    last_err = Some(GitError::new(
                        GitErrorKind::GitNotFound,
                        format!(
                            "git {} at {} is too old; Echo needs git {}.{} or newer",
                            fmt_version(version),
                            exe.display(),
                            MIN_GIT_VERSION.0,
                            MIN_GIT_VERSION.1
                        ),
                    ))
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            GitError::new(
                GitErrorKind::GitNotFound,
                match override_path {
                    Some(p) => format!("git not found at {}", p.display()),
                    None => "git was not found. Install git, or set its path in Settings.".into(),
                },
            )
        }))
    }

    pub fn path(&self) -> &Path {
        &self.exe
    }

    pub fn version_string(&self) -> String {
        fmt_version(self.version)
    }

    /// Run git and return raw output; only spawn failures and timeouts are errors.
    pub async fn exec(&self, repo: &Path, args: &[&str], mode: Mode) -> Result<Output, GitError> {
        let mut cmd = Command::new(&self.exe);
        cmd.arg("-C")
            .arg(repo)
            .args(["-c", "core.quotePath=false", "-c", "color.ui=false"])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if mode == Mode::Read {
            cmd.env("GIT_OPTIONAL_LOCKS", "0");
        }
        no_window(&mut cmd);

        let timeout = if mode == Mode::Network { NETWORK_TIMEOUT } else { LOCAL_TIMEOUT };
        let child = cmd
            .spawn()
            .map_err(|e| GitError::new(GitErrorKind::GitNotFound, format!("could not run git: {e}")))?;
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Err(_) => Err(GitError::new(
                if mode == Mode::Network { GitErrorKind::Network } else { GitErrorKind::Other },
                format!("git {} timed out after {}s", args.first().unwrap_or(&""), timeout.as_secs()),
            )),
            Ok(Err(e)) => Err(GitError::other(format!("git failed: {e}"))),
            Ok(Ok(out)) => Ok(Output {
                code: out.status.code(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }),
        }
    }

    /// Run git; non-zero exit becomes a classified [`GitError`].
    /// Write commands retry on a held lock (1 s, 2 s, 4 s).
    pub async fn run(&self, repo: &Path, args: &[&str], mode: Mode) -> Result<String, GitError> {
        let mut delays = [1u64, 2, 4].into_iter();
        loop {
            let out = self.exec(repo, args, mode).await?;
            if out.success() {
                return Ok(out.stdout);
            }
            let err = classify(&out.stderr);
            if err.kind == GitErrorKind::LockHeld && mode != Mode::Read {
                if let Some(d) = delays.next() {
                    log::debug!("git lock held in {}, retrying in {d}s", repo.display());
                    tokio::time::sleep(Duration::from_secs(d)).await;
                    continue;
                }
            }
            return Err(err);
        }
    }

    /// Run a command whose exit code is a boolean answer (0 = true, 1 = false).
    pub async fn check(&self, repo: &Path, args: &[&str]) -> Result<bool, GitError> {
        let out = self.exec(repo, args, Mode::Read).await?;
        match out.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(classify(&out.stderr)),
        }
    }
}

fn fmt_version(v: (u32, u32, u32)) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

async fn probe_version(exe: &Path) -> Result<(u32, u32, u32), GitError> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    no_window(&mut cmd);
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .map_err(|_| GitError::new(GitErrorKind::GitNotFound, "git --version timed out"))?
        .map_err(|e| GitError::new(GitErrorKind::GitNotFound, format!("could not run {}: {e}", exe.display())))?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_version(&text).ok_or_else(|| {
        GitError::new(GitErrorKind::GitNotFound, format!("{} is not a working git ({})", exe.display(), text.trim()))
    })
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

fn default_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let exe_name = if cfg!(windows) { "git.exe" } else { "git" };
    if cfg!(target_os = "macos") {
        // GUI apps don't inherit the shell PATH; prefer Homebrew, then the system shim.
        for p in ["/opt/homebrew/bin/git", "/usr/local/bin/git"] {
            out.push(PathBuf::from(p));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            out.push(dir.join(exe_name));
        }
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(base) = std::env::var_os(var) {
                out.push(PathBuf::from(base).join("Git").join("cmd").join("git.exe"));
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(local).join("Programs").join("Git").join("cmd").join("git.exe"));
        }
    } else {
        out.push(PathBuf::from("/usr/bin/git"));
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// Parse `git --version` output: "git version 2.43.0", "2.39.3 (Apple Git-146)",
/// "2.42.0.windows.1".
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let rest = s.trim().strip_prefix("git version ")?;
    let mut parts = rest.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// Parse `git ls-remote --heads origin` into `(branch, sha)` pairs.
pub fn parse_ls_remote(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|line| {
            let (sha, r) = line.split_once('\t')?;
            let name = r.trim().strip_prefix("refs/heads/")?;
            Some((name.to_string(), sha.trim().to_string()))
        })
        .collect()
}

/// One remote-tracking branch as read by `for-each-ref`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefInfo {
    pub name: String,
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub committed_at: i64,
}

/// Format string for [`parse_for_each_ref`].
pub const FOR_EACH_REF_FORMAT: &str =
    "--format=%(refname:lstrip=3)%00%(objectname)%00%(subject)%00%(authorname)%00%(committerdate:unix)";

/// Parse `for-each-ref refs/remotes/origin` output; skips the symbolic `HEAD`.
pub fn parse_for_each_ref(out: &str) -> Vec<RefInfo> {
    out.lines()
        .filter_map(|line| {
            let mut f = line.split('\0');
            let name = f.next()?.to_string();
            let sha = f.next()?.to_string();
            let subject = f.next().unwrap_or("").to_string();
            let author = f.next().unwrap_or("").to_string();
            let committed_at = f.next().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
            if name == "HEAD" || name.is_empty() || sha.is_empty() {
                return None;
            }
            Some(RefInfo { name, sha, subject, author, committed_at })
        })
        .collect()
}

/// Parse `git status --porcelain=v1 -z` into the list of paths.
pub fn parse_status_z(out: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let (xy, path) = entry.split_at(3);
        files.push(path.to_string());
        let xy = xy.as_bytes();
        if matches!(xy[0], b'R' | b'C') || matches!(xy[1], b'R' | b'C') {
            fields.next(); // original path of a rename/copy
        }
    }
    files
}

/// Extract the file list from "...would be overwritten by merge/checkout:" errors.
/// Returns `(paths, untracked)`.
pub fn parse_would_overwrite(stderr: &str) -> Option<(Vec<String>, bool)> {
    let mut lines = stderr.lines();
    let header = lines.by_ref().find(|l| l.contains("would be overwritten by"))?;
    let untracked = header.contains("untracked working tree files");
    let paths = lines
        .take_while(|l| l.starts_with('\t') || l.starts_with("        "))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Some((paths, untracked))
}

/// Classify git stderr into a [`GitError`].
pub fn classify(stderr: &str) -> GitError {
    let s = stderr;
    let lower = s.to_ascii_lowercase();
    let message = summarize(s);
    let kind = if let Some((paths, untracked)) = parse_would_overwrite(s) {
        GitErrorKind::WouldOverwrite { paths, untracked }
    } else if lower.contains("index.lock") || (lower.contains("unable to create") && lower.contains(".lock")) {
        GitErrorKind::LockHeld
    } else if lower.contains("authentication failed")
        || lower.contains("could not read username")
        || lower.contains("could not read password")
        || lower.contains("permission denied (publickey")
        || lower.contains("terminal prompts disabled")
        || lower.contains("repository not found")
        || lower.contains("host key verification failed")
        || lower.contains("returned error: 403")
    {
        GitErrorKind::Auth
    } else if lower.contains("could not resolve host")
        || lower.contains("could not resolve hostname")
        || lower.contains("connection timed out")
        || lower.contains("operation timed out")
        || lower.contains("connection refused")
        || lower.contains("network is unreachable")
        || lower.contains("failed to connect")
        || lower.contains("unable to access")
        || lower.contains("connection reset")
        || lower.contains("the remote end hung up unexpectedly")
    {
        GitErrorKind::Network
    } else if lower.contains("not possible to fast-forward") || lower.contains("not a fast-forward") {
        GitErrorKind::NotFastForward
    } else if lower.contains("couldn't find remote ref") {
        GitErrorKind::RemoteGone
    } else if lower.contains("not a git repository") {
        GitErrorKind::NotARepo
    } else {
        GitErrorKind::Other
    };
    GitError { kind, message }
}

/// First meaningful line(s) of stderr, for display.
fn summarize(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("hint:"))
        .take(3)
        .collect();
    if lines.is_empty() {
        "git failed".into()
    } else {
        lines.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("git version 2.43.0\n"), Some((2, 43, 0)));
        assert_eq!(parse_version("git version 2.39.3 (Apple Git-146)"), Some((2, 39, 3)));
        assert_eq!(parse_version("git version 2.42.0.windows.1"), Some((2, 42, 0)));
        assert_eq!(parse_version("git version 2.23"), Some((2, 23, 0)));
        assert_eq!(parse_version("nope"), None);
    }

    #[test]
    fn ls_remote_parsing() {
        let out = "aaa\trefs/heads/main\nbbb\trefs/heads/claude/foo-bar\nccc\trefs/tags/v1\n";
        assert_eq!(
            parse_ls_remote(out),
            vec![("main".into(), "aaa".into()), ("claude/foo-bar".into(), "bbb".into())]
        );
        assert!(parse_ls_remote("").is_empty());
    }

    #[test]
    fn for_each_ref_parsing() {
        let out = "HEAD\x00abc\x00subj\x00me\x00100\nmain\x00abc\x00Initial commit\x00Miles\x001700000000\nclaude/x\x00def\x00Fix: a\tb\x00Claude\x001700000100\n";
        let refs = parse_for_each_ref(out);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "main");
        assert_eq!(refs[0].committed_at, 1_700_000_000);
        assert_eq!(refs[1].subject, "Fix: a\tb");
        assert_eq!(refs[1].author, "Claude");
    }

    #[test]
    fn status_parsing_handles_renames() {
        let out = " M src/a.ts\0R  new.ts\0old.ts\0A  added file.txt\0";
        assert_eq!(parse_status_z(out), vec!["src/a.ts", "new.ts", "added file.txt"]);
        assert!(parse_status_z("").is_empty());
    }

    #[test]
    fn would_overwrite_parsing() {
        let err = "error: The following untracked working tree files would be overwritten by merge:\n\tfoo.txt\n\tdir/bar.txt\nPlease move or remove them before you merge.\nAborting\n";
        assert_eq!(
            parse_would_overwrite(err),
            Some((vec!["foo.txt".into(), "dir/bar.txt".into()], true))
        );
        let err = "error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt\nPlease commit your changes or stash them before you switch branches.\nAborting\n";
        assert_eq!(parse_would_overwrite(err), Some((vec!["a.txt".into()], false)));
    }

    #[test]
    fn classification() {
        let k = |s: &str| classify(s).kind;
        assert_eq!(
            k("fatal: Unable to create '/r/.git/index.lock': File exists.\n\nAnother git process seems to be running"),
            GitErrorKind::LockHeld
        );
        assert_eq!(k("fatal: Authentication failed for 'https://github.com/x/y.git/'"), GitErrorKind::Auth);
        assert_eq!(
            k("fatal: could not read Username for 'https://github.com': terminal prompts disabled"),
            GitErrorKind::Auth
        );
        assert_eq!(k("git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository."), GitErrorKind::Auth);
        assert_eq!(
            k("fatal: unable to access 'https://github.com/x/': Could not resolve host: github.com"),
            GitErrorKind::Network
        );
        assert_eq!(k("ssh: Could not resolve hostname github.com: nodename nor servname provided"), GitErrorKind::Network);
        assert_eq!(k("fatal: Not possible to fast-forward, aborting."), GitErrorKind::NotFastForward);
        assert_eq!(k("fatal: couldn't find remote ref claude/gone"), GitErrorKind::RemoteGone);
        assert_eq!(k("fatal: not a git repository (or any of the parent directories): .git"), GitErrorKind::NotARepo);
        assert!(matches!(
            k("error: The following untracked working tree files would be overwritten by checkout:\n\tx\nPlease move or remove them before you switch branches.\n"),
            GitErrorKind::WouldOverwrite { untracked: true, .. }
        ));
        assert_eq!(k("something odd"), GitErrorKind::Other);
    }

    #[test]
    fn summary_skips_hints() {
        let e = classify("hint: foo\nhint: bar\nfatal: real problem\n");
        assert_eq!(e.message, "fatal: real problem");
    }
}

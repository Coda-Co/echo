//! One dev server per repo, run in a pseudo-terminal so colours, prompts and
//! keypress shortcuts (Vite's `r`, `o`, ...) work as they do in a terminal.
//!
//! Output is kept as scrollback so the UI can re-attach after switching repos
//! or reloading, and Echo writes marker lines into it when it pulls, so a
//! hot-reload or crash can be tied to the push that caused it.
//!
//! Stopping kills the whole process tree (the process group on macOS/Linux,
//! `taskkill /T` on Windows): dev servers start child processes (esbuild,
//! workers, agents) that would otherwise keep the port bound.

use crate::core::{CoreEvent, EventSink};
use crate::store::now_ms;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// Scrollback kept per repo, in bytes of text.
const SCROLLBACK_BYTES: usize = 1 << 20;
/// How long a stopped server gets to exit before it is killed outright.
const STOP_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DevServerStatus {
    /// Not started since Echo launched.
    Idle,
    #[serde(rename_all = "camelCase")]
    Running {
        command: String,
        /// Unix millis.
        started_at: i64,
        /// First local URL the server printed (`http://localhost:5173/`).
        url: Option<String>,
    },
    /// Stop requested; waiting for the process tree to exit.
    #[serde(rename_all = "camelCase")]
    Stopping { command: String, started_at: i64, url: Option<String> },
    #[serde(rename_all = "camelCase")]
    Exited {
        command: String,
        started_at: i64,
        ended_at: i64,
        exit_code: Option<u32>,
        success: bool,
        /// Ended by Stop or Restart rather than by itself.
        stopped_by_user: bool,
    },
}

impl DevServerStatus {
    pub fn command(&self) -> Option<&str> {
        match self {
            DevServerStatus::Idle => None,
            DevServerStatus::Running { command, .. }
            | DevServerStatus::Stopping { command, .. }
            | DevServerStatus::Exited { command, .. } => Some(command),
        }
    }

    /// Running or stopping: a process exists.
    pub fn is_live(&self) -> bool {
        matches!(self, DevServerStatus::Running { .. } | DevServerStatus::Stopping { .. })
    }

    pub fn url(&self) -> Option<&str> {
        match self {
            DevServerStatus::Running { url, .. } | DevServerStatus::Stopping { url, .. } => url.as_deref(),
            _ => None,
        }
    }
}

/// Status plus scrollback, for a view attaching to a repo's terminal.
/// `seq` is the last output chunk included; later `DevServerOutput` events
/// with a higher `seq` follow on from it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DevServerSnapshot {
    pub status: DevServerStatus,
    pub output: String,
    pub seq: u64,
}

#[derive(Default)]
struct Scrollback {
    chunks: VecDeque<String>,
    bytes: usize,
    seq: u64,
    at_line_start: bool,
}

impl Scrollback {
    fn new() -> Self {
        Self { at_line_start: true, ..Default::default() }
    }

    fn push(&mut self, s: String) -> u64 {
        if s.is_empty() {
            return self.seq;
        }
        self.at_line_start = s.ends_with('\n');
        self.bytes += s.len();
        self.chunks.push_back(s);
        self.seq += 1;
        while self.bytes > SCROLLBACK_BYTES && self.chunks.len() > 1 {
            if let Some(c) = self.chunks.pop_front() {
                self.bytes -= c.len();
            }
        }
        self.seq
    }

    fn text(&self) -> String {
        let mut s = String::with_capacity(self.bytes);
        for c in &self.chunks {
            s.push_str(c);
        }
        s
    }
}

struct Run {
    id: u64,
    pid: Option<u32>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    stop_requested: bool,
}

struct Entry {
    status: DevServerStatus,
    log: Scrollback,
    run: Option<Run>,
    next_run: u64,
    /// Text since the last newline, for spotting the server's URL.
    partial_line: String,
}

impl Entry {
    fn new() -> Self {
        Self {
            status: DevServerStatus::Idle,
            log: Scrollback::new(),
            run: None,
            next_run: 1,
            partial_line: String::new(),
        }
    }
}

struct Shared {
    entries: Mutex<HashMap<String, Entry>>,
    sink: Arc<dyn EventSink>,
    env: tokio::sync::OnceCell<Option<Vec<(String, String)>>>,
}

/// Cheap to clone handle to every repo's dev server.
#[derive(Clone)]
pub struct DevServers(Arc<Shared>);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

type Events = Vec<CoreEvent>;

impl DevServers {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self(Arc::new(Shared { entries: Mutex::new(HashMap::new()), sink, env: tokio::sync::OnceCell::new() }))
    }

    /// Read the login shell's environment in the background, so the first
    /// Start doesn't wait for it. Call from within a Tokio runtime.
    pub fn warm_up(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            me.0.env.get_or_init(login_env).await;
        });
    }

    fn emit_all(&self, events: Events) {
        for e in events {
            self.0.sink.emit(e);
        }
    }

    pub fn status(&self, id: &str) -> DevServerStatus {
        lock(&self.0.entries).get(id).map(|e| e.status.clone()).unwrap_or(DevServerStatus::Idle)
    }

    pub fn snapshot(&self, id: &str) -> DevServerSnapshot {
        let entries = lock(&self.0.entries);
        match entries.get(id) {
            Some(e) => DevServerSnapshot { status: e.status.clone(), output: e.log.text(), seq: e.log.seq },
            None => DevServerSnapshot { status: DevServerStatus::Idle, output: String::new(), seq: 0 },
        }
    }

    /// Start `command` in `cwd`. Fails if this repo's server is already running.
    pub async fn start(
        &self,
        id: &str,
        cwd: &Path,
        command: &str,
        cols: u16,
        rows: u16,
    ) -> Result<DevServerStatus, String> {
        if self.status(id).is_live() {
            return Err("The dev server is already running. Stop it first.".into());
        }
        let env = self.0.env.get_or_init(login_env).await.clone();
        let size = PtySize { rows: rows.max(2), cols: cols.max(10), pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| format!("could not open a terminal: {e}"))?;
        let cmd = build_command(cwd, command, env.as_deref());
        let child = pair.slave.spawn_command(cmd).map_err(|e| format!("could not start “{command}”: {e}"))?;
        // Only the child holds the terminal's other end, so the reader ends
        // when every process using it has exited.
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let pid = child.process_id();
        let killer = child.clone_killer();

        let mut events = Events::new();
        let (run_id, status) = {
            let mut entries = lock(&self.0.entries);
            let e = entries.entry(id.to_string()).or_insert_with(Entry::new);
            if e.status.is_live() {
                // Lost a race with another Start; drop this one.
                let mut k = killer;
                let _ = k.kill();
                return Err("The dev server is already running.".into());
            }
            let run_id = e.next_run;
            e.next_run += 1;
            e.partial_line.clear();
            let mut banner = String::new();
            if e.log.seq > 0 {
                banner.push_str(if e.log.at_line_start { "\r\n" } else { "\r\n\r\n" });
            }
            banner.push_str(&format!("\x1b[2m$ {command}\x1b[0m\r\n"));
            let seq = e.log.push(banner.clone());
            events.push(CoreEvent::DevServerOutput { repo_id: id.to_string(), seq, data: banner });
            e.status = DevServerStatus::Running { command: command.to_string(), started_at: now_ms(), url: None };
            e.run = Some(Run { id: run_id, pid, master: pair.master, writer, killer, stop_requested: false });
            events.push(CoreEvent::DevServerChanged { repo_id: id.to_string(), status: e.status.clone() });
            (run_id, e.status.clone())
        };
        self.emit_all(events);
        log::info!("dev server started for {id}: {command} (pid {pid:?})");

        let (done_tx, done_rx) = mpsc::channel::<()>();
        let me = self.clone();
        let rid = id.to_string();
        std::thread::Builder::new()
            .name("devserver-read".into())
            .spawn(move || me.read_loop(&rid, run_id, reader, done_tx))
            .map_err(|e| e.to_string())?;
        let me = self.clone();
        let rid = id.to_string();
        let mut child = child;
        std::thread::Builder::new()
            .name("devserver-wait".into())
            .spawn(move || {
                let exit = child.wait();
                // Let the reader drain what the process printed last.
                let _ = done_rx.recv_timeout(Duration::from_millis(1000));
                me.finish(&rid, run_id, exit.ok());
            })
            .map_err(|e| e.to_string())?;
        Ok(status)
    }

    fn read_loop(&self, id: &str, run_id: u64, mut reader: Box<dyn Read + Send>, done: mpsc::Sender<()>) {
        let mut buf = [0u8; 16 * 1024];
        let mut pending = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = decode_utf8(&mut pending, &buf[..n]);
                    self.output(id, run_id, text);
                }
            }
        }
        if !pending.is_empty() {
            self.output(id, run_id, String::from_utf8_lossy(&pending).into_owned());
        }
        let _ = done.send(());
    }

    fn output(&self, id: &str, run_id: u64, text: String) {
        if text.is_empty() {
            return;
        }
        let mut events = Events::new();
        {
            let mut entries = lock(&self.0.entries);
            let Some(e) = entries.get_mut(id) else { return };
            // Late output from a run that has been replaced by a restart.
            if e.next_run != run_id + 1 {
                return;
            }
            if let DevServerStatus::Running { url: url @ None, .. } = &mut e.status {
                e.partial_line.push_str(&text);
                if let Some(found) = find_local_url_in(&mut e.partial_line) {
                    *url = Some(found);
                    events.push(CoreEvent::DevServerChanged { repo_id: id.to_string(), status: e.status.clone() });
                }
            }
            let seq = e.log.push(text.clone());
            events.insert(0, CoreEvent::DevServerOutput { repo_id: id.to_string(), seq, data: text });
        }
        self.emit_all(events);
    }

    fn finish(&self, id: &str, run_id: u64, exit: Option<portable_pty::ExitStatus>) {
        let mut events = Events::new();
        let pid = {
            let mut entries = lock(&self.0.entries);
            let Some(e) = entries.get_mut(id) else { return };
            let Some(run) = e.run.take_if(|r| r.id == run_id) else { return };
            let (command, started_at) = match &e.status {
                DevServerStatus::Running { command, started_at, .. }
                | DevServerStatus::Stopping { command, started_at, .. } => (command.clone(), *started_at),
                _ => (String::new(), now_ms()),
            };
            let ended_at = now_ms();
            let signal = exit.as_ref().and_then(|x| x.signal()).map(str::to_string);
            // A process ended by a signal has no real exit code.
            let code = exit.as_ref().filter(|_| signal.is_none()).map(|x| x.exit_code());
            let success = exit.as_ref().is_some_and(|x| x.success());
            let ran = format_duration(ended_at - started_at);
            // Ctrl+C typed into the terminal is a deliberate stop, not a crash.
            let interrupted =
                code.is_some_and(is_interrupt_code) || signal.as_deref().is_some_and(|s| s.starts_with("Interrupt"));
            let stopped = run.stop_requested || interrupted;
            let (color, what) = if stopped {
                ("2", "stopped".to_string())
            } else if success {
                ("2", "exited".to_string())
            } else {
                (
                    "31",
                    match (code, &signal) {
                        (Some(c), _) => format!("exited with code {c}"),
                        (None, Some(sig)) => format!("ended by signal: {sig}"),
                        (None, None) => "exited".to_string(),
                    },
                )
            };
            let lead = if e.log.at_line_start { "\r\n" } else { "\r\n\r\n" };
            let line = format!("{lead}\x1b[{color}m── {what} · {} · ran {ran} ──\x1b[0m\r\n", local_time());
            let seq = e.log.push(line.clone());
            events.push(CoreEvent::DevServerOutput { repo_id: id.to_string(), seq, data: line });
            e.status = DevServerStatus::Exited {
                command,
                started_at,
                ended_at,
                exit_code: code,
                success,
                stopped_by_user: stopped,
            };
            events.push(CoreEvent::DevServerChanged { repo_id: id.to_string(), status: e.status.clone() });
            log::info!("dev server for {id} {what}");
            run.pid
            // `run` drops here, closing the terminal.
        };
        // Anything the server left behind (workers, agents) holds the port.
        if let Some(pid) = pid {
            terminate_tree(pid, true);
        }
        self.emit_all(events);
    }

    /// Ask the server to stop; it is killed outright after a grace period.
    pub fn stop(&self, id: &str) -> Result<DevServerStatus, String> {
        let mut events = Events::new();
        let (pid, run_id, status) = {
            let mut entries = lock(&self.0.entries);
            let Some(e) = entries.get_mut(id) else { return Ok(DevServerStatus::Idle) };
            let Some(run) = e.run.as_mut() else { return Ok(e.status.clone()) };
            run.stop_requested = true;
            if let DevServerStatus::Running { command, started_at, url } = &e.status {
                e.status =
                    DevServerStatus::Stopping { command: command.clone(), started_at: *started_at, url: url.clone() };
                events.push(CoreEvent::DevServerChanged { repo_id: id.to_string(), status: e.status.clone() });
            }
            (run.pid, run.id, e.status.clone())
        };
        self.emit_all(events);
        match pid {
            Some(pid) => terminate_tree(pid, false),
            None => self.kill_run(id, run_id),
        }
        let me = self.clone();
        let rid = id.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(STOP_GRACE);
            let still = lock(&me.0.entries).get(&rid).and_then(|e| e.run.as_ref()).is_some_and(|r| r.id == run_id);
            if still {
                log::warn!("dev server for {rid} did not stop in time; killing it");
                if let Some(pid) = pid {
                    terminate_tree(pid, true);
                }
                me.kill_run(&rid, run_id);
            }
        });
        Ok(status)
    }

    fn kill_run(&self, id: &str, run_id: u64) {
        let mut entries = lock(&self.0.entries);
        if let Some(run) = entries.get_mut(id).and_then(|e| e.run.as_mut()).filter(|r| r.id == run_id) {
            let _ = run.killer.kill();
        }
    }

    /// Wait until the server is no longer running (or `timeout` passes).
    pub async fn wait_stopped(&self, id: &str, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while self.status(id).is_live() {
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        true
    }

    /// Stop (if running) and start again with the same command.
    pub async fn restart(&self, id: &str, cwd: &Path, cols: u16, rows: u16) -> Result<DevServerStatus, String> {
        let command = self.status(id).command().map(str::to_string).ok_or("Nothing to restart yet.")?;
        if self.status(id).is_live() {
            self.stop(id)?;
            if !self.wait_stopped(id, STOP_GRACE + Duration::from_secs(2)).await {
                return Err("The dev server did not stop.".into());
            }
        }
        self.start(id, cwd, &command, cols, rows).await
    }

    /// Keystrokes from the terminal view.
    pub fn write(&self, id: &str, data: &str) -> Result<(), String> {
        let mut entries = lock(&self.0.entries);
        let Some(run) = entries.get_mut(id).and_then(|e| e.run.as_mut()) else { return Ok(()) };
        run.writer.write_all(data.as_bytes()).and_then(|_| run.writer.flush()).map_err(|e| e.to_string())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let entries = lock(&self.0.entries);
        let Some(run) = entries.get(id).and_then(|e| e.run.as_ref()) else { return Ok(()) };
        run.master
            .resize(PtySize { rows: rows.max(2), cols: cols.max(10), pixel_width: 0, pixel_height: 0 })
            .map_err(|e| e.to_string())
    }

    /// Write an Echo marker line (`── echo · pulled 2 commits … ──`) into a
    /// running server's output.
    pub fn marker(&self, id: &str, text: &str) {
        let mut events = Events::new();
        {
            let mut entries = lock(&self.0.entries);
            let Some(e) = entries.get_mut(id) else { return };
            if !matches!(e.status, DevServerStatus::Running { .. }) {
                return;
            }
            let lead = if e.log.at_line_start { "" } else { "\r\n" };
            let line = format!("{lead}\x1b[94m── echo · {text} · {} ──\x1b[0m\r\n", local_time());
            let seq = e.log.push(line.clone());
            events.push(CoreEvent::DevServerOutput { repo_id: id.to_string(), seq, data: line });
        }
        self.emit_all(events);
    }

    /// Stop the server and drop its scrollback (repo removed).
    pub fn forget(&self, id: &str) {
        let _ = self.stop(id);
        let me = self.clone();
        let rid = id.to_string();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + STOP_GRACE + Duration::from_secs(2);
            while me.status(&rid).is_live() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            lock(&me.0.entries).remove(&rid);
        });
    }

    /// Stop every server before Echo quits. Blocks for at most about a second.
    pub fn shutdown(&self) {
        let pids: Vec<u32> = {
            let mut entries = lock(&self.0.entries);
            entries
                .values_mut()
                .filter_map(|e| e.run.as_mut())
                .filter_map(|r| {
                    r.stop_requested = true;
                    r.pid
                })
                .collect()
        };
        if pids.is_empty() {
            return;
        }
        for &pid in &pids {
            terminate_tree(pid, false);
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(1000);
        while std::time::Instant::now() < deadline && lock(&self.0.entries).values().any(|e| e.run.is_some()) {
            std::thread::sleep(Duration::from_millis(50));
        }
        for pid in pids {
            terminate_tree(pid, true);
        }
        let mut entries = lock(&self.0.entries);
        for e in entries.values_mut() {
            if let Some(mut r) = e.run.take() {
                let _ = r.killer.kill();
            }
        }
    }
}

#[cfg(unix)]
fn build_command(cwd: &Path, command: &str, env: Option<&[(String, String)]>) -> CommandBuilder {
    let shell =
        env.and_then(|e| e.iter().find(|(k, _)| k == "SHELL").map(|(_, v)| v.clone())).unwrap_or_else(user_shell);
    let mut cmd = CommandBuilder::new(&shell);
    // Non-interactive, so the shell runs the command without job control and
    // everything stays in one process group that Stop can kill.
    cmd.arg("-c");
    cmd.arg(command);
    cmd.cwd(cwd);
    if let Some(env) = env {
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd
}

#[cfg(windows)]
fn build_command(cwd: &Path, command: &str, _env: Option<&[(String, String)]>) -> CommandBuilder {
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
    let mut cmd = CommandBuilder::new(comspec);
    cmd.args(["/D", "/S", "/C"]);
    cmd.arg(command);
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd
}

#[cfg(unix)]
fn user_shell() -> String {
    if let Some(s) = std::env::var("SHELL").ok().filter(|s| !s.is_empty()) {
        return s;
    }
    // SAFETY: getpwuid returns a pointer into static storage or null; we copy
    // the string out immediately.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if !pw.is_null() && !(*pw).pw_shell.is_null() {
            if let Ok(s) = std::ffi::CStr::from_ptr((*pw).pw_shell).to_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    if cfg!(target_os = "macos") {
        "/bin/zsh".into()
    } else {
        "/bin/sh".into()
    }
}

/// The environment of the user's login shell. Apps opened from the Dock or
/// Finder don't get the PATH set up in `.zshrc`/`.zprofile` (nvm, pnpm,
/// Homebrew), so it is read once from an interactive login shell.
#[cfg(unix)]
async fn login_env() -> Option<Vec<(String, String)>> {
    const MARK: &[u8] = b"__ECHO_ENV_START__\n";
    let shell = user_shell();
    let run = tokio::process::Command::new(&shell)
        .args(["-l", "-i", "-c", "printf '\\n__ECHO_ENV_START__\\n'; /usr/bin/env -0"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = match tokio::time::timeout(Duration::from_secs(10), run).await {
        Ok(Ok(o)) => o.stdout,
        Ok(Err(e)) => {
            log::warn!("could not read the login shell environment from {shell}: {e}");
            return None;
        }
        Err(_) => {
            log::warn!("reading the login shell environment from {shell} timed out");
            return None;
        }
    };
    let start = out.windows(MARK.len()).position(|w| w == MARK)? + MARK.len();
    let env = parse_env(&out[start..]);
    if env.iter().any(|(k, _)| k == "PATH") {
        Some(env)
    } else {
        None
    }
}

#[cfg(windows)]
async fn login_env() -> Option<Vec<(String, String)>> {
    None
}

/// Parse `env -0` output, dropping variables that describe the probe shell.
#[cfg(any(unix, test))]
fn parse_env(bytes: &[u8]) -> Vec<(String, String)> {
    const SKIP: &[&str] = &["_", "SHLVL", "PWD", "OLDPWD", "PS1", "PS2", "PROMPT", "RPROMPT"];
    bytes
        .split(|b| *b == 0)
        .filter_map(|kv| {
            let s = String::from_utf8_lossy(kv);
            let (k, v) = s.split_once('=')?;
            let k = k.trim_start_matches('\n');
            let valid = !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.starts_with(|c: char| c.is_ascii_digit());
            (valid && !SKIP.contains(&k)).then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// Signal a server's whole process tree. `force` kills outright.
#[cfg(unix)]
fn terminate_tree(pid: u32, force: bool) {
    let sig = if force { libc::SIGKILL } else { libc::SIGTERM };
    // The pty child is a session and process-group leader, so its pid is the
    // group id; children it started are in the same group.
    // SAFETY: plain syscall with integer arguments.
    unsafe {
        libc::killpg(pid as libc::pid_t, sig);
    }
}

#[cfg(windows)]
fn terminate_tree(pid: u32, _force: bool) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

/// Decode `chunk` as UTF-8, carrying an incomplete trailing sequence over to
/// the next call.
fn decode_utf8(pending: &mut Vec<u8>, chunk: &[u8]) -> String {
    pending.extend_from_slice(chunk);
    let mut out = String::new();
    loop {
        match std::str::from_utf8(pending) {
            Ok(s) => {
                out.push_str(s);
                pending.clear();
                return out;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                out.push_str(std::str::from_utf8(&pending[..valid]).expect("valid prefix"));
                match e.error_len() {
                    None => {
                        pending.drain(..valid);
                        return out;
                    }
                    Some(n) => {
                        out.push('\u{FFFD}');
                        pending.drain(..valid + n);
                    }
                }
            }
        }
    }
}

/// Remove ANSI escape sequences.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Look through the complete lines in `partial` for a local URL, keeping only
/// the unfinished last line.
fn find_local_url_in(partial: &mut String) -> Option<String> {
    let Some(nl) = partial.rfind('\n') else {
        if partial.len() > 4096 {
            partial.clear();
        }
        return None;
    };
    let complete: String = partial.drain(..=nl).collect();
    strip_ansi(&complete).lines().find_map(local_url)
}

/// `http://localhost:5173/` from a line such as `➜  Local:   http://localhost:5173/`.
pub fn local_url(line: &str) -> Option<String> {
    for (i, _) in line.match_indices("http") {
        let rest = &line[i..];
        let Some(after) = rest.strip_prefix("http://").or_else(|| rest.strip_prefix("https://")) else { continue };
        let scheme = &rest[..rest.len() - after.len()];
        for host in ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"] {
            let Some(port_part) = after.strip_prefix(host).and_then(|r| r.strip_prefix(':')) else { continue };
            let digits: String = port_part.chars().take_while(|c| c.is_ascii_digit()).collect();
            let Ok(port) = digits.parse::<u16>() else { continue };
            if port == 0 {
                continue;
            }
            let path: String = port_part[digits.len()..]
                .chars()
                .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | ')' | ']' | '>' | ','))
                .collect();
            let path = if path.starts_with('/') { path } else { "/".into() };
            let host = if host == "0.0.0.0" { "localhost" } else { host };
            return Some(format!("{scheme}{host}:{port}{path}"));
        }
    }
    None
}

/// True for the URLs [`local_url`] produces, so only those are opened.
pub fn is_local_url(url: &str) -> bool {
    local_url(url).as_deref() == Some(url)
}

/// Exit codes of a process ended by Ctrl+C: 130 from a Unix shell,
/// STATUS_CONTROL_C_EXIT on Windows.
fn is_interrupt_code(code: u32) -> bool {
    code == 130 || code == 0xC000_013A
}

fn local_time() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

fn format_duration(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        _ => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_split_across_reads() {
        let mut pending = Vec::new();
        let bytes = "a➜b".as_bytes();
        assert_eq!(decode_utf8(&mut pending, &bytes[..2]), "a");
        assert_eq!(pending.len(), 1);
        assert_eq!(decode_utf8(&mut pending, &bytes[2..]), "➜b");
        assert!(pending.is_empty());
        assert_eq!(decode_utf8(&mut pending, b"x\xffy"), "x\u{FFFD}y");
    }

    #[test]
    fn scrollback_trims_oldest() {
        let mut s = Scrollback::new();
        let big = "x".repeat(SCROLLBACK_BYTES / 2 + 1);
        s.push(big.clone());
        s.push(big.clone());
        s.push("tail\n".into());
        assert_eq!(s.seq, 3);
        assert!(s.bytes <= SCROLLBACK_BYTES);
        assert!(s.text().ends_with("tail\n"));
        assert!(s.at_line_start);
    }

    #[test]
    fn finds_local_urls() {
        assert_eq!(local_url("  ➜  Local:   http://localhost:5173/").as_deref(), Some("http://localhost:5173/"));
        assert_eq!(
            local_url("ready - started server on 0.0.0.0:3000, url: http://0.0.0.0:3000").as_deref(),
            Some("http://localhost:3000/")
        );
        assert_eq!(
            local_url("Listening on https://127.0.0.1:8443/app).").as_deref(),
            Some("https://127.0.0.1:8443/app")
        );
        assert_eq!(local_url("see https://example.com:443/"), None);
        assert_eq!(local_url("http://localhost:99999"), None);
        assert!(is_local_url("http://localhost:5173/"));
        assert!(!is_local_url("http://localhost:5173/ && rm -rf"));
        assert!(!is_local_url("file:///etc/passwd"));

        let mut partial = "\x1b[32m➜\x1b[39m  Local:   \x1b[36mhttp://localhost:\x1b[1m5173\x1b[22m/".to_string();
        assert_eq!(find_local_url_in(&mut partial), None);
        partial.push_str("\x1b[39m\r\n  next");
        assert_eq!(find_local_url_in(&mut partial).as_deref(), Some("http://localhost:5173/"));
        assert_eq!(partial, "  next");
    }

    #[test]
    fn strips_ansi() {
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m \x1b]0;title\x07done"), "ok done");
    }

    #[test]
    fn parses_env() {
        let env = parse_env(b"PATH=/a:/b\0_=/usr/bin/env\0MULTI=one\ntwo\0SHLVL=2\0\n");
        assert_eq!(env, vec![("PATH".into(), "/a:/b".into()), ("MULTI".into(), "one\ntwo".into())]);
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(45_000), "45s");
        assert_eq!(format_duration(16 * 60_000 + 5_000), "16m");
        assert_eq!(format_duration(63 * 60_000), "1h 3m");
    }
}

#[cfg(all(test, unix))]
mod process_tests {
    #[tokio::test(flavor = "multi_thread")]
    async fn ctrl_c_counts_as_stopped() {
        let dev = servers();
        let dir = tempfile::tempdir().unwrap();
        dev.start("r", dir.path(), "echo ready; sleep 300", 80, 24).await.unwrap();
        until("ready", Duration::from_secs(15), || dev.snapshot("r").output.contains("ready\r\n")).await;
        dev.write("r", "\x03").unwrap();
        assert!(dev.wait_stopped("r", Duration::from_secs(10)).await);
        match dev.status("r") {
            DevServerStatus::Exited { stopped_by_user, .. } => assert!(stopped_by_user),
            s => panic!("unexpected {s:?}"),
        }
        assert!(dev.snapshot("r").output.contains("── stopped"));
    }

    use super::*;
    use std::time::Instant;

    struct Collect(Mutex<Vec<CoreEvent>>);
    impl EventSink for Collect {
        fn emit(&self, event: CoreEvent) {
            lock(&self.0).push(event);
        }
    }

    fn servers() -> DevServers {
        DevServers::new(Arc::new(Collect(Mutex::new(Vec::new()))))
    }

    async fn until(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while !f() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runs_detects_url_and_stops_the_whole_tree() {
        let dev = servers();
        let dir = tempfile::tempdir().unwrap();
        let cmd = "sleep 300 & echo child=$!; echo '  ➜  Local:   http://localhost:5173/'; wait";
        dev.start("r", dir.path(), cmd, 80, 24).await.unwrap();
        assert!(dev.start("r", dir.path(), cmd, 80, 24).await.is_err(), "second start refused");

        until("url", Duration::from_secs(15), || dev.status("r").url().is_some()).await;
        assert_eq!(dev.status("r").url(), Some("http://localhost:5173/"));
        let out = dev.snapshot("r").output;
        assert!(out.contains(&format!("$ {cmd}")));
        let child: i32 = out.rsplit("child=").next().unwrap().lines().next().unwrap().trim().parse().unwrap();
        assert!(alive(child));

        dev.marker("r", "pulled 1 commit on claude/x");
        assert!(dev.snapshot("r").output.contains("── echo · pulled 1 commit on claude/x"));

        dev.stop("r").unwrap();
        assert!(dev.wait_stopped("r", Duration::from_secs(10)).await);
        match dev.status("r") {
            DevServerStatus::Exited { stopped_by_user, .. } => assert!(stopped_by_user),
            s => panic!("unexpected {s:?}"),
        }
        until("child gone", Duration::from_secs(5), || !alive(child)).await;
        assert!(dev.snapshot("r").output.contains("── stopped"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_exit_code_input_and_restart() {
        let dev = servers();
        let dir = tempfile::tempdir().unwrap();
        dev.start("r", dir.path(), "read line; echo got=$line; exit 3", 80, 24).await.unwrap();
        dev.write("r", "hello\r").unwrap();
        assert!(dev.wait_stopped("r", Duration::from_secs(15)).await);
        let snap = dev.snapshot("r");
        assert!(snap.output.contains("got=hello"), "{}", snap.output);
        assert!(snap.output.contains("exited with code 3"));
        match snap.status {
            DevServerStatus::Exited { exit_code, success, stopped_by_user, .. } => {
                assert_eq!(exit_code, Some(3));
                assert!(!success);
                assert!(!stopped_by_user);
            }
            s => panic!("unexpected {s:?}"),
        }

        // Restart reuses the exact command, appending to the same scrollback.
        let before = snap.seq;
        dev.restart("r", dir.path(), 80, 24).await.unwrap();
        dev.write("r", "again\r").unwrap();
        assert!(dev.wait_stopped("r", Duration::from_secs(15)).await);
        let snap = dev.snapshot("r");
        assert!(snap.seq > before);
        assert!(snap.output.contains("got=again"));
        assert_eq!(snap.status.command(), Some("read line; echo got=$line; exit 3"));
    }
}

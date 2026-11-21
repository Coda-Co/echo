//! "Open in Finder/Explorer", "Open in terminal", and links in the browser.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

fn spawn(cmd: &mut Command) -> io::Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

#[cfg(target_os = "macos")]
pub fn open_in_file_manager(path: &Path) -> io::Result<()> {
    spawn(Command::new("open").arg(path))
}

#[cfg(target_os = "macos")]
pub fn open_in_terminal(path: &Path) -> io::Result<()> {
    spawn(Command::new("open").args(["-a", "Terminal"]).arg(path))
}

#[cfg(windows)]
pub fn open_in_file_manager(path: &Path) -> io::Result<()> {
    spawn(Command::new("explorer").arg(path))
}

#[cfg(windows)]
pub fn open_in_terminal(path: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Prefer Windows Terminal, fall back to a plain cmd window.
    if spawn(Command::new("wt").arg("-d").arg(path)).is_ok() {
        return Ok(());
    }
    spawn(Command::new("cmd").args(["/C", "start", "", "/D"]).arg(path).arg("cmd.exe").creation_flags(CREATE_NO_WINDOW))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn open_in_file_manager(path: &Path) -> io::Result<()> {
    spawn(Command::new("xdg-open").arg(path))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn open_in_terminal(path: &Path) -> io::Result<()> {
    spawn(Command::new("x-terminal-emulator").current_dir(path))
}

/// Open an https URL in the default browser. Callers validate the URL.
#[cfg(target_os = "macos")]
pub fn open_url(url: &str) -> io::Result<()> {
    spawn(Command::new("open").arg(url))
}

#[cfg(windows)]
pub fn open_url(url: &str) -> io::Result<()> {
    // No shell involved, so `&` in query strings is passed through untouched.
    spawn(Command::new("rundll32").arg("url.dll,FileProtocolHandler").arg(url))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn open_url(url: &str) -> io::Result<()> {
    spawn(Command::new("xdg-open").arg(url))
}

/// Windows: if Echo was started by a process that turned on Redirection Trust
/// (for example the installer's "Run Echo" step, or the installer the updater
/// runs), Echo and every command it runs refuse junctions created without
/// admin rights. pnpm's `node_modules` and Node version managers are full of
/// them, so dev servers fail with "The path cannot be traversed because it
/// contains an untrusted mount point". The restriction can't be lifted from
/// inside the process, so start a fresh copy through Explorer (as the Start
/// menu would) and exit. Returns true when the caller should exit.
///
/// Must run before anything else in `main`, so this copy never takes the
/// single-instance lock that the fresh copy needs.
#[cfg(windows)]
pub fn relaunch_if_redirection_restricted() -> bool {
    use std::time::{Duration, SystemTime};

    if !redirection_trust_enforced() {
        return false;
    }
    // Never loop: if a relaunch happened moments ago and the fresh copy is
    // still restricted, carry on as is.
    let marker = std::env::temp_dir().join("echo-relaunched");
    let recent = std::fs::metadata(&marker)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < Duration::from_secs(30));
    if recent {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let _ = std::fs::write(&marker, b"");
    Command::new("explorer.exe").arg(exe).spawn().is_ok()
}

#[cfg(not(windows))]
pub fn relaunch_if_redirection_restricted() -> bool {
    false
}

#[cfg(windows)]
fn redirection_trust_enforced() -> bool {
    // PROCESS_MITIGATION_POLICY::ProcessRedirectionTrustPolicy (Windows 11 22H2+).
    const PROCESS_REDIRECTION_TRUST_POLICY: i32 = 16;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
        fn GetProcessMitigationPolicy(
            process: *mut core::ffi::c_void,
            policy: i32,
            buffer: *mut core::ffi::c_void,
            length: usize,
        ) -> i32;
    }
    // PROCESS_MITIGATION_REDIRECTION_TRUST_POLICY: bit 0 EnforceRedirectionTrust,
    // bit 1 AuditRedirectionTrust.
    let mut flags: u32 = 0;
    // SAFETY: a 4-byte buffer for a 4-byte policy struct; older Windows
    // versions return 0 (failure) and we treat that as not enforced.
    let ok = unsafe {
        GetProcessMitigationPolicy(
            GetCurrentProcess(),
            PROCESS_REDIRECTION_TRUST_POLICY,
            &mut flags as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>(),
        )
    };
    ok != 0 && flags & 1 != 0
}

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

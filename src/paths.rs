//! Small filesystem helpers shared by the modules that write state.

use crate::Result;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Write a file readable only by its owner.
pub(crate) fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    write_with_mode(path, contents, 0o600)
}

/// Write an owner-only executable script.
pub(crate) fn write_executable(path: &Path, contents: &[u8]) -> Result<()> {
    write_with_mode(path, contents, 0o700)
}

fn write_with_mode(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true).mode(mode);
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from(PermissionsMode(mode)))?;
    Ok(())
}

struct PermissionsMode(u32);

impl From<PermissionsMode> for fs::Permissions {
    fn from(mode: PermissionsMode) -> Self {
        use std::os::unix::fs::PermissionsExt;
        fs::Permissions::from_mode(mode.0)
    }
}

/// Locate an executable on `PATH`.
pub(crate) fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// Read a pid file and report whether that process is alive.
pub(crate) fn pid_alive(pid_file: &Path) -> Option<i32> {
    let pid: i32 = fs::read_to_string(pid_file).ok()?.trim().parse().ok()?;
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
        .is_ok()
        .then_some(pid)
}

/// The last `lines` lines of a text file, or an empty string.
pub(crate) fn tail(path: &Path, lines: usize) -> String {
    let contents = fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = contents.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

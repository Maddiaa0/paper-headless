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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Temp;
    use std::os::unix::fs::PermissionsExt;
    mod when_writing_state {
        use super::*;
        #[test]
        fn creates_private_parented_files() {
            let t = Temp::new();
            let p = t.path.join("nested/state");
            write_private_file(&p, b"secret").unwrap();
            assert_eq!(fs::read(&p).unwrap(), b"secret");
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        #[test]
        fn restricts_existing_file_permissions() {
            let t = Temp::new();
            let p = t.path.join("state");
            fs::write(&p, "long previous value").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o666)).unwrap();
            write_private_file(&p, b"new").unwrap();
            assert_eq!(fs::read(&p).unwrap(), b"new");
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        #[test]
        fn creates_executable_scripts() {
            let t = Temp::new();
            let p = t.path.join("script");
            write_executable(&p, b"#!/bin/sh\nexit 0\n").unwrap();
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert!(
                std::process::Command::new("/bin/sh")
                    .arg(&p)
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
    mod when_reading_logs {
        use super::*;
        #[test]
        fn returns_the_last_lines() {
            let t = Temp::new();
            let p = t.path.join("log");
            fs::write(&p, "a\nb\nc\n").unwrap();
            assert_eq!(tail(&p, 2), "b\nc");
            assert_eq!(tail(&p, 99), "a\nb\nc");
            assert_eq!(tail(&p, 0), "");
        }
        #[test]
        fn handles_missing_and_empty_files() {
            let t = Temp::new();
            let p = t.path.join("log");
            assert_eq!(tail(&p, 10), "");
            fs::write(&p, "").unwrap();
            assert_eq!(tail(&p, 10), "");
        }
    }
    mod when_reading_a_pid_file {
        use super::*;
        #[test]
        fn recognizes_the_current_process() {
            let t = Temp::new();
            let p = t.path.join("pid");
            fs::write(&p, format!("{}\n", std::process::id())).unwrap();
            assert_eq!(pid_alive(&p), Some(std::process::id() as i32));
        }
        #[test]
        fn rejects_malformed_and_missing_files() {
            let t = Temp::new();
            let p = t.path.join("pid");
            assert!(pid_alive(&p).is_none());
            fs::write(&p, "not a pid").unwrap();
            assert!(pid_alive(&p).is_none());
        }
    }
    mod when_locating_a_program {
        use super::*;
        #[test]
        fn finds_an_existing_absolute_path() {
            let t = Temp::new();
            let p = t.script("program", "exit 0");
            assert_eq!(which(p.to_str().unwrap()), Some(p));
        }
        #[test]
        fn rejects_a_missing_program() {
            let t = Temp::new();
            assert!(which(t.path.join("missing").to_str().unwrap()).is_none());
        }
    }
}

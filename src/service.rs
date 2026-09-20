//! systemd management. A user unit by default (with lingering so it survives
//! logout); `--system` writes a system unit that runs as the invoking user.

use crate::settings::{Settings, home_dir};
use crate::{Error, Result, paths};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub(crate) const UNIT_NAME: &str = "paper-headless.service";

#[derive(Clone, Copy)]
pub(crate) enum Scope {
    User,
    System,
}

impl Scope {
    pub(crate) fn from_flag(system: bool) -> Self {
        if system { Self::System } else { Self::User }
    }

    fn unit_path(self) -> Result<PathBuf> {
        Ok(match self {
            Self::User => env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or(home_dir()?.join(".config"))
                .join("systemd")
                .join("user")
                .join(UNIT_NAME),
            Self::System => PathBuf::from("/etc/systemd/system").join(UNIT_NAME),
        })
    }

    fn systemctl(self, arguments: &[&str]) -> Result<()> {
        let mut command = Command::new("systemctl");
        if matches!(self, Self::User) {
            command.arg("--user");
        }
        let output = command
            .args(arguments)
            .output()
            .map_err(|error| Error::msg(format!("could not run systemctl: {error}")))?;
        if output.status.success() {
            return Ok(());
        }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(Error::msg(if detail.is_empty() {
            format!(
                "systemctl {} failed with {}",
                arguments.join(" "),
                output.status
            )
        } else {
            format!("systemctl {} failed: {detail}", arguments.join(" "))
        }))
    }

    pub(crate) fn is_installed(self) -> bool {
        self.unit_path().is_ok_and(|path| path.exists())
    }

    pub(crate) fn is_active(self) -> bool {
        self.systemctl(&["is-active", "--quiet", UNIT_NAME]).is_ok()
    }
}

fn unit_contents(settings: &Settings, scope: Scope, binary: &std::path::Path) -> Result<String> {
    let executable = binary
        .to_str()
        .ok_or_else(|| Error::msg("paper-headless executable path is not UTF-8"))?;
    let mut lines = vec![
        "[Unit]".to_owned(),
        format!(
            "Description=Paper Desktop headless runner (MCP on {})",
            settings.mcp_url()
        ),
        "After=network-online.target".to_owned(),
        "Wants=network-online.target".to_owned(),
        String::new(),
        "[Service]".to_owned(),
        "Type=simple".to_owned(),
        format!("ExecStart=\"{}\" serve", executable.replace('"', "\\\"")),
        "Restart=on-failure".to_owned(),
        "RestartSec=5".to_owned(),
        "TimeoutStopSec=25".to_owned(),
        "KillMode=mixed".to_owned(),
    ];
    if matches!(scope, Scope::System) {
        let user = env::var("SUDO_USER")
            .or_else(|_| env::var("USER"))
            .unwrap_or_else(|_| "root".into());
        lines.push(format!("User={user}"));
        lines.push(format!("Environment=HOME={}", home_dir()?.display()));
    }
    lines.push(format!(
        "Environment=PATH={}",
        env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into())
    ));
    lines.extend(
        settings
            .environment_lines()
            .into_iter()
            .map(|line| format!("Environment={line}")),
    );
    lines.push(String::new());
    lines.push("[Install]".to_owned());
    lines.push(match scope {
        Scope::User => "WantedBy=default.target".to_owned(),
        Scope::System => "WantedBy=multi-user.target".to_owned(),
    });
    lines.push(String::new());
    Ok(lines.join("\n"))
}

pub(crate) fn install(settings: &Settings, scope: Scope) -> Result<()> {
    crate::paper::ensure_binary(settings)?;
    let binary = env::current_exe()?;
    let path = scope.unit_path()?;
    paths::write_private_file(&path, unit_contents(settings, scope, &binary)?.as_bytes())?;
    fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o644))?;
    println!("wrote {}", path.display());
    if matches!(scope, Scope::User) {
        // Keep the user manager (and Paper) alive after logout.
        if let Ok(user) = env::var("USER") {
            let _ = Command::new("loginctl")
                .args(["enable-linger", &user])
                .output();
        }
    }
    scope.systemctl(&["daemon-reload"])?;
    scope.systemctl(&["enable", "--now", UNIT_NAME])?;
    println!("service enabled and started ({})", unit_label(scope));
    Ok(())
}

pub(crate) fn uninstall(settings: &Settings, scope: Scope, purge: bool) -> Result<()> {
    if scope.is_installed() {
        let _ = scope.systemctl(&["disable", "--now", UNIT_NAME]);
        let path = scope.unit_path()?;
        fs::remove_file(&path)?;
        let _ = scope.systemctl(&["daemon-reload"]);
        println!("removed {}", path.display());
    } else {
        println!("service was not installed ({})", unit_label(scope));
    }
    if purge {
        if settings.data_dir.exists() {
            fs::remove_dir_all(&settings.data_dir)?;
            println!(
                "removed {} (Paper profile and login)",
                settings.data_dir.display()
            );
        }
    } else {
        println!(
            "kept {} (use --purge to delete the Paper login)",
            settings.data_dir.display()
        );
    }
    Ok(())
}

pub(crate) fn start(scope: Scope) -> Result<()> {
    require_installed(scope)?;
    scope.systemctl(&["start", UNIT_NAME])
}

pub(crate) fn stop(scope: Scope) -> Result<()> {
    require_installed(scope)?;
    scope.systemctl(&["stop", UNIT_NAME])
}

pub(crate) fn restart(scope: Scope) -> Result<()> {
    require_installed(scope)?;
    scope.systemctl(&["restart", UNIT_NAME])
}

fn require_installed(scope: Scope) -> Result<()> {
    if scope.is_installed() {
        return Ok(());
    }
    Err(Error::msg(format!(
        "the {} service is not installed; run `paper-headless install` first \
         (or run `paper-headless serve` in a terminal)",
        unit_label(scope)
    )))
}

pub(crate) fn unit_label(scope: Scope) -> &'static str {
    match scope {
        Scope::User => "systemd --user unit",
        Scope::System => "systemd system unit",
    }
}

pub(crate) fn journal(scope: Scope, lines: usize, follow: bool) -> Result<()> {
    let mut command = Command::new("journalctl");
    if matches!(scope, Scope::User) {
        command.arg("--user");
    }
    command.args(["-u", UNIT_NAME, "-n", &lines.to_string(), "--no-pager"]);
    if follow {
        command.arg("-f");
    }
    let status = command
        .status()
        .map_err(|error| Error::msg(format!("could not run journalctl: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::msg(format!("journalctl exited with {status}")))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Temp, settings};
    mod when_generating_a_unit {
        use super::*;
        #[test]
        fn uses_the_user_service_target() {
            let t = Temp::new();
            let s = settings(&t);
            let unit = unit_contents(&s, Scope::User, std::path::Path::new("/bin/paper-headless"))
                .unwrap();
            assert!(unit.contains("WantedBy=default.target"));
            assert!(unit.contains("KillMode=mixed"));
            assert!(unit.contains("Restart=on-failure"));
            for line in s.environment_lines() {
                assert!(unit.contains(&format!("Environment={line}")));
            }
            assert!(!unit.contains("\nUser="));
        }
        #[test]
        fn uses_the_system_service_target() {
            let t = Temp::new();
            let unit = unit_contents(
                &settings(&t),
                Scope::System,
                std::path::Path::new("/bin/paper-headless"),
            )
            .unwrap();
            assert!(unit.contains("WantedBy=multi-user.target"));
            assert!(unit.contains("\nUser="));
            assert!(unit.contains("Environment=HOME="));
        }
        #[test]
        fn quotes_the_executable_path() {
            let t = Temp::new();
            let unit = unit_contents(
                &settings(&t),
                Scope::User,
                std::path::Path::new("/tmp/my tool"),
            )
            .unwrap();
            assert!(unit.contains("ExecStart=\"/tmp/my tool\" serve"));
        }
        #[test]
        fn rejects_a_nonutf8_executable() {
            use std::os::unix::ffi::OsStringExt;
            let t = Temp::new();
            let p = PathBuf::from(std::ffi::OsString::from_vec(vec![255]));
            assert!(unit_contents(&settings(&t), Scope::User, &p).is_err());
        }
    }
}

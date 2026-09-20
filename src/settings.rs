//! Where state lives and how Paper is reached. Every value comes from a
//! global CLI flag with an environment-variable fallback, so the systemd
//! unit can persist the same choices as `Environment=` lines.

use crate::{Error, Result};
use std::env;
use std::path::PathBuf;

pub(crate) const DEFAULT_DISPLAY: &str = ":99";
pub(crate) const DEFAULT_SCREEN: &str = "1600x1000x24";
pub(crate) const DEFAULT_CDP_PORT: u16 = 9222;
/// Paper Desktop hard-codes its MCP listener to this port.
pub(crate) const DEFAULT_MCP_PORT: u16 = 29979;
const DEFAULT_PAPER_BINARY: &str = "/opt/Paper/paper-desktop";

#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub(crate) data_dir: PathBuf,
    pub(crate) display: String,
    pub(crate) screen: String,
    pub(crate) cdp_port: u16,
    pub(crate) mcp_port: u16,
    pub(crate) paper_binary: PathBuf,
}

impl Settings {
    pub(crate) fn resolve(
        data_dir: Option<PathBuf>,
        display: String,
        screen: String,
        cdp_port: u16,
        mcp_port: u16,
        paper_binary: Option<PathBuf>,
    ) -> Result<Self> {
        let data_dir = match data_dir {
            Some(directory) => directory,
            None => default_data_dir()?,
        };
        display_number(&display)?;
        let paper_binary = match paper_binary {
            Some(binary) => binary,
            None => locate_paper_binary(),
        };
        Ok(Self {
            data_dir,
            display,
            screen,
            cdp_port,
            mcp_port,
            paper_binary,
        })
    }

    pub(crate) fn profile_dir(&self) -> PathBuf {
        self.data_dir.join("profile")
    }

    pub(crate) fn shim_dir(&self) -> PathBuf {
        self.data_dir.join("bin")
    }

    pub(crate) fn auth_url_file(&self) -> PathBuf {
        self.data_dir.join("auth-url.txt")
    }

    pub(crate) fn paper_log(&self) -> PathBuf {
        self.data_dir.join("paper.log")
    }

    pub(crate) fn xvfb_log(&self) -> PathBuf {
        self.data_dir.join("xvfb.log")
    }

    pub(crate) fn paper_pid_file(&self) -> PathBuf {
        self.data_dir.join("paper.pid")
    }

    pub(crate) fn serve_pid_file(&self) -> PathBuf {
        self.data_dir.join("serve.pid")
    }

    pub(crate) fn dbus_socket(&self) -> PathBuf {
        self.data_dir.join("dbus.sock")
    }

    pub(crate) fn mcp_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.mcp_port)
    }

    pub(crate) fn cdp_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.cdp_port)
    }

    pub(crate) fn display_number(&self) -> u32 {
        display_number(&self.display).unwrap_or(99)
    }

    pub(crate) fn x_socket(&self) -> PathBuf {
        PathBuf::from(format!("/tmp/.X11-unix/X{}", self.display_number()))
    }

    /// `Environment=` lines that make a systemd unit reproduce these settings.
    pub(crate) fn environment_lines(&self) -> Vec<String> {
        vec![
            format!("PAPER_HEADLESS_DATA_DIR={}", self.data_dir.display()),
            format!("PAPER_HEADLESS_DISPLAY={}", self.display),
            format!("PAPER_HEADLESS_SCREEN={}", self.screen),
            format!("PAPER_HEADLESS_CDP_PORT={}", self.cdp_port),
            format!("PAPER_HEADLESS_MCP_PORT={}", self.mcp_port),
            format!("PAPER_DESKTOP_BIN={}", self.paper_binary.display()),
        ]
    }
}

pub(crate) fn home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::msg("HOME is not set"))
}

fn default_data_dir() -> Result<PathBuf> {
    let base = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".local").join("share"));
    Ok(base.join("paper-headless"))
}

pub(crate) fn locate_paper_binary() -> PathBuf {
    let default = PathBuf::from(DEFAULT_PAPER_BINARY);
    if default.is_file() {
        return default;
    }
    crate::paths::which("paper-desktop").unwrap_or(default)
}

pub(crate) fn display_number(display: &str) -> Result<u32> {
    display
        .strip_prefix(':')
        .and_then(|rest| rest.split('.').next())
        .and_then(|number| number.parse().ok())
        .ok_or_else(|| Error::msg(format!("invalid X display {display:?}; expected e.g. :99")))
}

#[cfg(test)]
mod tests {
    use super::*;

    mod when_reading_a_display {
        use super::*;
        #[test]
        fn reads_the_number() {
            assert_eq!(display_number(":99").unwrap(), 99);
        }
        #[test]
        fn drops_the_screen_suffix() {
            assert_eq!(display_number(":1.0").unwrap(), 1);
        }
        #[test]
        fn rejects_a_display_with_no_colon() {
            assert!(display_number("99").is_err());
        }
        #[test]
        fn rejects_a_nonnumeric_display() {
            for value in [":", ":abc", ":-1"] {
                assert!(display_number(value).is_err());
            }
        }
    }
    mod when_resolving_settings {
        use super::*;
        #[test]
        fn preserves_explicit_overrides() {
            let s = Settings::resolve(
                Some("/tmp/custom".into()),
                ":12".into(),
                "800x600x24".into(),
                9001,
                9002,
                Some("/bin/true".into()),
            )
            .unwrap();
            assert_eq!(s.display_number(), 12);
            assert_eq!(s.screen, "800x600x24");
            assert_eq!(s.paper_binary, PathBuf::from("/bin/true"));
            assert_eq!(s.cdp_port, 9001);
            assert_eq!(s.mcp_port, 9002);
        }
        #[test]
        fn rejects_an_invalid_display() {
            assert!(
                Settings::resolve(
                    Some("/tmp/custom".into()),
                    "invalid".into(),
                    "800x600x24".into(),
                    9001,
                    9002,
                    Some("/bin/true".into())
                )
                .is_err()
            );
        }
        #[test]
        fn keeps_state_paths_under_the_data_directory() {
            let t = crate::test_support::Temp::new();
            let s = crate::test_support::settings(&t);
            for path in [
                s.profile_dir(),
                s.shim_dir(),
                s.auth_url_file(),
                s.paper_log(),
                s.xvfb_log(),
                s.paper_pid_file(),
                s.serve_pid_file(),
                s.dbus_socket(),
            ] {
                assert!(path.starts_with(&t.path));
            }
            assert_eq!(s.x_socket(), PathBuf::from("/tmp/.X11-unix/X99"));
        }
        #[test]
        fn binds_endpoint_urls_to_loopback() {
            let t = crate::test_support::Temp::new();
            let s = crate::test_support::settings(&t);
            assert_eq!(s.cdp_url(), "http://127.0.0.1:9222");
            assert_eq!(s.mcp_url(), "http://127.0.0.1:29979/mcp");
        }
        #[test]
        fn persists_settings_into_the_service_environment() {
            let t = crate::test_support::Temp::new();
            let s = crate::test_support::settings(&t);
            let lines = s.environment_lines();
            assert!(lines.contains(&format!("PAPER_HEADLESS_DATA_DIR={}", t.path.display())));
            assert!(lines.contains(&"PAPER_HEADLESS_DISPLAY=:99".into()));
            assert!(lines.contains(&"PAPER_HEADLESS_CDP_PORT=9222".into()));
            assert!(lines.contains(&"PAPER_HEADLESS_MCP_PORT=29979".into()));
            assert!(lines.contains(&format!("PAPER_DESKTOP_BIN={}", s.paper_binary.display())));
        }
    }
}

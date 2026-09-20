mod support;
use std::fs;
use std::process::{Command, Output};
use support::{Temp, http};
struct Fixture {
    temp: Temp,
}
impl Fixture {
    fn new() -> Self {
        let temp = Temp::new();
        temp.script("paper", "exit 0");
        // Record calls without inheriting the real user's service or agent configuration.
        for name in ["systemctl", "loginctl", "claude", "codex", "journalctl"] {
            temp.script(
                &format!("bin/{name}"),
                &format!("{{ printf '{name}'; printf ' %s' \"$@\"; printf '\\n'; }} >> \"$CALLS\""),
            );
        }
        Self { temp }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_paper-headless"));
        c.env_clear()
            .env("HOME", self.temp.path.join("home"))
            .env("USER", "test-user")
            .env("XDG_CONFIG_HOME", self.temp.path.join("config"))
            .env("PATH", self.temp.path.join("bin"))
            .env("CALLS", self.temp.path.join("calls"))
            .env("PAPER_HEADLESS_CDP_PORT", "0")
            .env("PAPER_HEADLESS_MCP_PORT", "0")
            .arg("--data-dir")
            .arg(self.temp.path.join("data"))
            .arg("--paper-binary")
            .arg(self.temp.path.join("paper"));
        c
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn unit(&self) -> std::path::PathBuf {
        self.temp
            .path
            .join("config/systemd/user/paper-headless.service")
    }
    fn calls(&self) -> String {
        fs::read_to_string(self.temp.path.join("calls")).unwrap_or_default()
    }
    fn install(&self) {
        assert!(self.run(&["install", "--skip-agents"]).status.success());
    }
}
fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
fn port(url: &str) -> &str {
    url.rsplit(':').next().unwrap()
}

mod when_using_the_cli {
    use super::*;
    #[test]
    fn lists_all_commands_in_help() {
        let f = Fixture::new();
        let out = f.run(&["--help"]);
        assert!(out.status.success());
        for name in [
            "serve",
            "install",
            "uninstall",
            "start",
            "stop",
            "restart",
            "status",
            "logs",
            "login",
            "login-code",
            "check",
            "configure-agents",
            "doctor",
        ] {
            assert!(stdout(&out).contains(name), "{name}");
        }
    }
    #[test]
    fn rejects_unknown_commands() {
        let f = Fixture::new();
        let out = f.run(&["unknown-command"]);
        assert_eq!(out.status.code(), Some(2));
        assert!(stderr(&out).contains("unrecognized subcommand"));
    }
    #[test]
    fn rejects_invalid_ports() {
        let out = Command::new(env!("CARGO_BIN_EXE_paper-headless"))
            .args(["--cdp-port", "65536", "check"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(stderr(&out).contains("invalid value"));
    }
    #[test]
    fn rejects_an_invalid_display() {
        let f = Fixture::new();
        let out = f.run(&["--display", "invalid", "check"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(stderr(&out).contains("invalid X display"));
    }
    #[test]
    fn reads_logs_from_the_selected_data_directory() {
        let f = Fixture::new();
        let dir = f.temp.path.join("data");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("paper.log"), "first\nsecond\nthird\n").unwrap();
        let out = f.run(&["logs", "-n", "2"]);
        assert!(out.status.success());
        assert_eq!(stdout(&out), "second\nthird\n");
    }
    #[test]
    fn reports_missing_logs() {
        let f = Fixture::new();
        let out = f.run(&["logs"]);
        assert!(out.status.success());
        assert!(stdout(&out).contains("no output yet"));
    }
}
mod when_managing_the_service {
    use super::*;
    #[test]
    fn installs_a_user_unit_and_registers_agents() {
        let f = Fixture::new();
        assert!(f.run(&["install"]).status.success());
        let unit = fs::read_to_string(f.unit()).unwrap();
        assert!(unit.contains("WantedBy=default.target"));
        let calls = f.calls();
        assert!(calls.contains("systemctl --user daemon-reload"));
        assert!(calls.contains("systemctl --user enable --now paper-headless.service"));
        assert!(
            calls.contains(
                "claude mcp add --transport http --scope user paper http://127.0.0.1:0/mcp"
            )
        );
        assert!(calls.contains("codex mcp add paper --url http://127.0.0.1:0/mcp"));
    }
    #[test]
    fn skips_agent_registration_on_request() {
        let f = Fixture::new();
        f.install();
        assert!(!f.calls().contains("claude"));
        assert!(!f.calls().contains("codex"));
    }
    #[test]
    fn routes_start_stop_and_restart_to_systemctl() {
        let f = Fixture::new();
        f.install();
        for action in ["start", "stop", "restart"] {
            assert!(f.run(&[action]).status.success());
            assert!(
                f.calls()
                    .contains(&format!("systemctl --user {action} paper-headless.service"))
            );
        }
    }
    #[test]
    fn preserves_the_profile_on_uninstall() {
        let f = Fixture::new();
        f.install();
        let profile = f.temp.path.join("data/profile");
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("session"), "keep").unwrap();
        assert!(f.run(&["uninstall"]).status.success());
        assert!(!f.unit().exists());
        assert_eq!(fs::read_to_string(profile.join("session")).unwrap(), "keep");
        assert!(f.calls().contains("disable --now paper-headless.service"));
    }
    #[test]
    fn purges_the_profile_on_request() {
        let f = Fixture::new();
        f.install();
        let data = f.temp.path.join("data");
        fs::create_dir_all(data.join("profile")).unwrap();
        assert!(f.run(&["uninstall", "--purge"]).status.success());
        assert!(!data.exists());
    }
    #[test]
    fn rejects_start_without_installation() {
        let f = Fixture::new();
        let out = f.run(&["start"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(stderr(&out).contains("not installed"));
    }
    #[test]
    fn routes_journal_flags() {
        let f = Fixture::new();
        assert!(f.run(&["logs", "--follow", "-n", "7"]).status.success());
        assert!(
            f.calls()
                .contains("journalctl --user -u paper-headless.service -n 7 --no-pager -f")
        );
    }
}
mod when_diagnosing_an_isolated_installation {
    use super::*;
    #[test]
    fn reports_missing_dependencies_and_endpoints() {
        let f = Fixture::new();
        let out = f.run(&["doctor"]);
        assert!(out.status.success());
        let text = stdout(&out);
        assert!(text.contains("FAIL Xvfb"));
        assert!(text.contains("FAIL devtools"));
        assert!(text.contains("FAIL mcp port"));
    }
    #[test]
    fn reports_status_without_modifying_the_profile() {
        let f = Fixture::new();
        let out = f.run(&["status"]);
        assert!(out.status.success());
        assert!(stdout(&out).contains("not installed"));
        assert!(!f.temp.path.join("data").exists());
    }
}
mod when_configuring_agents {
    use super::*;
    #[test]
    fn registers_both_clients() {
        let f = Fixture::new();
        assert!(f.run(&["configure-agents"]).status.success());
        assert!(f.calls().contains("claude mcp add"));
        assert!(f.calls().contains("codex mcp add"));
    }
    #[test]
    fn removes_both_registrations() {
        let f = Fixture::new();
        assert!(f.run(&["configure-agents", "--remove"]).status.success());
        assert!(f.calls().contains("claude mcp remove --scope user paper"));
        assert!(f.calls().contains("codex mcp remove paper"));
    }
    #[test]
    fn prints_manual_setup_without_clients() {
        let f = Fixture::new();
        for name in ["claude", "codex"] {
            fs::remove_file(f.temp.path.join("bin").join(name)).unwrap();
        }
        let out = f.run(&["configure-agents"]);
        assert!(out.status.success());
        assert!(stdout(&out).contains("configure them by hand"));
    }
}
mod when_checking_authentication {
    use super::*;
    #[test]
    fn rejects_an_error_page_before_mcp() {
        let f = Fixture::new();
        let (url, w) = http(
            200,
            "",
            r#"[{"type":"page","url":"https://app.paper.design/error?message=failed"}]"#,
        );
        let out = f
            .command()
            .env("PAPER_HEADLESS_CDP_PORT", port(&url))
            .args(["check"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(
            stderr(&out).contains("Paper rejected sign-in"),
            "{}",
            stderr(&out)
        );
        w.join().unwrap();
    }
    #[test]
    fn rejects_background_pages_before_mcp() {
        let f = Fixture::new();
        let (url, w) = http(
            200,
            "",
            r#"[{"type":"page","url":"https://app.paper.design/static/desktop/preloader"}]"#,
        );
        let out = f
            .command()
            .env("PAPER_HEADLESS_CDP_PORT", port(&url))
            .arg("check")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(stderr(&out).contains("not signed in"), "{}", stderr(&out));
        w.join().unwrap();
    }
    #[test]
    fn accepts_a_signed_in_renderer_and_handshake() {
        let f = Fixture::new();
        let (cdp, cw) = http(
            200,
            "",
            r#"[{"type":"page","url":"https://app.paper.design/file/test"}]"#,
        );
        let (mcp, mw) = http(
            200,
            "mcp-session-id: test\r\n",
            r#"{"result":{"serverInfo":{"name":"paper-desktop","version":"test"}}}"#,
        );
        let out = f
            .command()
            .env("PAPER_HEADLESS_CDP_PORT", port(&cdp))
            .env("PAPER_HEADLESS_MCP_PORT", port(&mcp))
            .arg("check")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(stdout(&out).contains("MCP ready"));
        cw.join().unwrap();
        mw.join().unwrap();
    }
}

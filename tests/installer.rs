#[allow(dead_code)]
mod support;
use std::process::Command;
use support::Temp;
fn installer() -> Command {
    let mut c = Command::new("/bin/bash");
    c.arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"));
    c.env_clear()
        .env("HOME", "/nonexistent")
        .env("PATH", "/usr/bin:/bin");
    c
}

mod when_invoking_the_installer {
    use super::*;
    #[test]
    fn prints_help_without_installing() {
        let out = installer().arg("--help").output().unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("paper-headless"));
    }
    #[test]
    fn rejects_unknown_options_without_installing() {
        let out = installer().arg("--invalid").output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&out.stderr).contains("unknown option"));
    }
    #[test]
    fn rejects_unsupported_platforms() {
        let t = Temp::new();
        t.script("uname", "echo Darwin");
        let out = installer().env("PATH", &t.path).output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("only runs on Linux"));
    }
    #[test]
    fn rejects_unsupported_architectures() {
        let t = Temp::new();
        t.script(
            "uname",
            "if [ \"$1\" = -s ]; then echo Linux; else echo unsupported; fi",
        );
        t.script("id", "echo 0");
        let out = installer().env("PATH", &t.path).output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("unsupported architecture"));
    }
}

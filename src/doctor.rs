//! `doctor` and `status`: everything a person needs to see when the setup is
//! not working, in one screen.

use crate::auth::{SignInState, sign_in_state};
use crate::service::{Scope, unit_label};
use crate::settings::Settings;
use crate::{Result, mcp, paths, supervisor};

fn line(ok: bool, label: &str, detail: impl AsRef<str>) {
    println!(
        "{} {label:<22} {}",
        if ok { "ok  " } else { "FAIL" },
        detail.as_ref()
    );
}

fn optional(present: bool, label: &str, detail: &str) {
    println!(
        "{} {label:<22} {detail}",
        if present { "ok  " } else { "--  " }
    );
}

pub(crate) fn doctor(settings: &Settings, scope: Scope) -> Result<()> {
    println!("paper-headless {}", env!("CARGO_PKG_VERSION"));
    println!();
    line(
        settings.paper_binary.is_file(),
        "paper desktop",
        settings.paper_binary.display().to_string(),
    );
    line(
        paths::which("Xvfb").is_some(),
        "Xvfb",
        "virtual X server (apt install xvfb)",
    );
    optional(
        paths::which("dbus-daemon").is_some(),
        "dbus-daemon",
        "session bus, optional (apt install dbus)",
    );
    optional(
        paths::which("x11vnc").is_some(),
        "x11vnc",
        "optional, view the display over SSH (apt install x11vnc)",
    );
    line(
        settings.data_dir.exists() || std::fs::create_dir_all(&settings.data_dir).is_ok(),
        "data directory",
        settings.data_dir.display().to_string(),
    );
    println!();
    status(settings, scope)
}

pub(crate) fn status(settings: &Settings, scope: Scope) -> Result<()> {
    let installed = scope.is_installed();
    optional(
        installed,
        "service",
        &format!(
            "{} {}",
            unit_label(scope),
            if !installed {
                "not installed"
            } else if scope.is_active() {
                "active"
            } else {
                "inactive"
            }
        ),
    );
    match paths::pid_alive(&settings.serve_pid_file()) {
        Some(pid) => line(true, "serve process", format!("running (pid {pid})")),
        None => line(false, "serve process", "not running"),
    }
    match paths::pid_alive(&settings.paper_pid_file()) {
        Some(pid) => line(true, "paper process", format!("running (pid {pid})")),
        None => line(false, "paper process", "not running"),
    }
    let cdp_up = supervisor::port_open(settings.cdp_port);
    line(cdp_up, "devtools", settings.cdp_url());
    let mcp_up = supervisor::port_open(settings.mcp_port);
    line(mcp_up, "mcp port", settings.mcp_url());
    if cdp_up {
        match sign_in_state(settings) {
            Ok(SignInState::SignedIn) => line(true, "sign-in", "signed in"),
            Ok(SignInState::SignedOut) => {
                line(false, "sign-in", "signed out; run `paper-headless login`")
            }
            Ok(SignInState::Unknown) => line(false, "sign-in", "unknown (app still loading?)"),
            Err(error) => line(false, "sign-in", format!("could not query: {error}")),
        }
    }
    if mcp_up {
        match mcp::probe(&settings.mcp_url()) {
            Ok(probe) => line(probe.is_ready(), "mcp handshake", mcp::describe(&probe)),
            Err(error) => line(false, "mcp handshake", error.to_string()),
        }
    }
    Ok(())
}

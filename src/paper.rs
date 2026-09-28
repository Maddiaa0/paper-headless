//! Launching Paper Desktop: the Electron arguments a headless host needs, the
//! `xdg-open` shim that captures the sign-in URL instead of opening a
//! browser, and deep-link delivery through a second instance.

use crate::settings::Settings;
use crate::{Error, Result, paths};
use std::env;
use std::fs;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a second instance may take to hand its deep link to the primary
/// instance and exit. If it outlives this it became the primary itself.
const SECOND_INSTANCE_TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) fn ensure_binary(settings: &Settings) -> Result<()> {
    if settings.paper_binary.is_file() {
        return Ok(());
    }
    Err(Error::msg(format!(
        "Paper Desktop binary not found at {}; install the `paper` package \
         (see install.sh) or pass --paper-binary / PAPER_DESKTOP_BIN",
        settings.paper_binary.display()
    )))
}

/// Install a `PATH` shim so Electron's `shell.openExternal` records the URL it
/// wanted to open instead of launching a browser that does not exist here.
pub(crate) fn ensure_shim(settings: &Settings) -> Result<()> {
    let script = format!(
        "#!/bin/sh\n# Installed by paper-headless: records URLs Paper asks to open.\nprintf '%s\\n' \"$@\" >> '{}'\nexit 0\n",
        settings.auth_url_file().display()
    );
    paths::write_file(
        &settings.shim_dir().join("xdg-open"),
        script.as_bytes(),
        0o700,
    )
}

fn shimmed_path(settings: &Settings) -> String {
    let current = env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
    format!("{}:{current}", settings.shim_dir().display())
}

fn base_command(settings: &Settings, dbus_address: Option<&str>) -> Command {
    let mut command = Command::new(&settings.paper_binary);
    command
        .arg("--no-sandbox")
        .arg("--disable-gpu")
        .arg("--disable-dev-shm-usage")
        .arg(format!(
            "--user-data-dir={}",
            settings.profile_dir().display()
        ))
        .env("DISPLAY", &settings.display)
        .env("PATH", shimmed_path(settings))
        .env_remove("ELECTRON_RUN_AS_NODE")
        .stdin(Stdio::null());
    match dbus_address {
        Some(address) => command.env("DBUS_SESSION_BUS_ADDRESS", address),
        None => command.env_remove("DBUS_SESSION_BUS_ADDRESS"),
    };
    command
}

/// Start the primary Paper instance with DevTools on the loopback CDP port.
pub(crate) fn spawn_primary(settings: &Settings, dbus_address: Option<&str>) -> Result<Child> {
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(settings.paper_log())?;
    let mut command = base_command(settings, dbus_address);
    command
        .arg(format!("--remote-debugging-port={}", settings.cdp_port))
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    command
        .spawn()
        .map_err(|error| Error::msg(format!("could not start Paper: {error}")))
}

/// Deliver a `paper://` deep link to the running instance. Electron's
/// single-instance lock makes the new process forward its argv and exit.
pub(crate) fn send_deep_link(settings: &Settings, link: &str) -> Result<()> {
    let mut child = base_command(settings, None)
        .arg(link)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Error::msg(format!("could not start a second Paper instance: {error}")))?;
    let started = Instant::now();
    while started.elapsed() < SECOND_INSTANCE_TIMEOUT {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(Error::msg(
        "the second Paper instance did not exit, so no primary instance was running; \
         start the service first (`paper-headless start` or `paper-headless serve`)",
    ))
}

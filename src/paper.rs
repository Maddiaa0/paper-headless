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
    paths::write_executable(&settings.shim_dir().join("xdg-open"), script.as_bytes())
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Temp, settings};
    mod when_preparing_paper {
        use super::*;
        #[test]
        fn rejects_a_missing_binary() {
            let t = Temp::new();
            assert!(
                ensure_binary(&settings(&t))
                    .unwrap_err()
                    .to_string()
                    .contains("Paper Desktop binary not found")
            );
        }
        #[test]
        fn accepts_an_existing_binary() {
            let t = Temp::new();
            t.script("paper", "exit 0");
            assert!(ensure_binary(&settings(&t)).is_ok());
        }
        #[test]
        fn captures_browser_urls_through_the_shim() {
            let t = Temp::new();
            let s = settings(&t);
            ensure_shim(&s).unwrap();
            let shim = s.shim_dir().join("xdg-open");
            for url in [
                "https://example.com/?a=1&b=2",
                "paper://auth/callback?code=test",
            ] {
                assert!(
                    Command::new("/bin/sh")
                        .arg(&shim)
                        .arg(url)
                        .status()
                        .unwrap()
                        .success()
                );
            }
            assert_eq!(
                fs::read_to_string(s.auth_url_file()).unwrap(),
                "https://example.com/?a=1&b=2\npaper://auth/callback?code=test\n"
            );
        }
        #[test]
        fn isolates_the_launch_environment() {
            let t = Temp::new();
            let s = settings(&t);
            let c = base_command(&s, None);
            let args: Vec<_> = c.get_args().collect();
            assert!(args.contains(&std::ffi::OsStr::new("--no-sandbox")));
            assert!(args.contains(&std::ffi::OsStr::new("--disable-gpu")));
            let vars: std::collections::HashMap<_, _> = c.get_envs().collect();
            assert_eq!(vars[std::ffi::OsStr::new("ELECTRON_RUN_AS_NODE")], None);
            assert_eq!(vars[std::ffi::OsStr::new("DBUS_SESSION_BUS_ADDRESS")], None);
            assert_eq!(
                vars[std::ffi::OsStr::new("DISPLAY")],
                Some(std::ffi::OsStr::new(":99"))
            );
            let c = base_command(&s, Some("unix:path=/tmp/test-bus"));
            assert!(c.get_envs().any(|(k, v)| k == "DBUS_SESSION_BUS_ADDRESS"
                && v == Some(std::ffi::OsStr::new("unix:path=/tmp/test-bus"))));
        }
    }
    mod when_launching_paper {
        use super::*;
        #[test]
        fn passes_the_debugging_port_and_captures_logs() {
            let t = Temp::new();
            let s = settings(&t);
            t.script("paper", "printf '%s\n' \"$@\"; echo stderr >&2");
            let mut child = spawn_primary(&s, None).unwrap();
            assert!(child.wait().unwrap().success());
            let log = fs::read_to_string(s.paper_log()).unwrap();
            assert!(log.contains("--remote-debugging-port=9222"));
            assert!(log.contains("stderr"));
        }
        #[test]
        fn delivers_a_callback_to_a_second_instance() {
            let t = Temp::new();
            let s = settings(&t);
            let output = t.path.join("argv");
            t.script(
                "paper",
                &format!("printf '%s\n' \"$@\" > {}", output.display()),
            );
            send_deep_link(&s, "paper://auth/callback?code=test").unwrap();
            assert!(
                fs::read_to_string(output)
                    .unwrap()
                    .contains("paper://auth/callback?code=test")
            );
        }
        #[test]
        fn reports_a_failed_spawn() {
            let t = Temp::new();
            assert!(
                send_deep_link(&settings(&t), "paper://auth/callback?code=test")
                    .unwrap_err()
                    .to_string()
                    .contains("could not start a second Paper instance")
            );
        }
    }
}

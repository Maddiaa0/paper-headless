//! `serve`: bring up a virtual X display and a private session bus, then run
//! Paper Desktop under them and restart it if it dies. Everything it spawns is
//! a child, so a systemd stop tears the whole tree down.

use crate::settings::Settings;
use crate::{Error, Result, paper, paths};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use signal_hook::consts::{SIGINT, SIGTERM};
use std::fs;
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

const MCP_STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
const CRASH_WINDOW: Duration = Duration::from_secs(120);
const MAX_CRASHES_IN_WINDOW: usize = 5;
const RESTART_DELAY: Duration = Duration::from_secs(3);

pub(crate) fn serve(settings: &Settings) -> Result<()> {
    paper::ensure_binary(settings)?;
    fs::create_dir_all(settings.profile_dir())?;
    paper::ensure_shim(settings)?;
    if let Some(pid) = paths::pid_alive(&settings.serve_pid_file()) {
        return Err(Error::msg(format!(
            "another `paper-headless serve` is already running (pid {pid})"
        )));
    }
    fs::write(settings.serve_pid_file(), std::process::id().to_string())?;
    // Fresh log per serve session; restarts within the session append.
    fs::write(settings.paper_log(), b"")?;

    let stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGTERM, Arc::clone(&stop))?;
    signal_hook::flag::register(SIGINT, Arc::clone(&stop))?;

    let mut xvfb = start_xvfb(settings)?;
    let mut dbus = start_dbus(settings)?;
    let dbus_address = dbus
        .as_ref()
        .map(|_| format!("unix:path={}", settings.dbus_socket().display()));

    let outcome = supervise(settings, dbus_address.as_deref(), &stop);

    if let Some(child) = dbus.as_mut() {
        terminate(child, Duration::from_secs(3));
    }
    if let Some(child) = xvfb.as_mut() {
        terminate(child, Duration::from_secs(3));
    }
    let _ = fs::remove_file(settings.paper_pid_file());
    let _ = fs::remove_file(settings.serve_pid_file());
    let _ = fs::remove_file(settings.dbus_socket());
    outcome
}

fn supervise(settings: &Settings, dbus_address: Option<&str>, stop: &AtomicBool) -> Result<()> {
    let mut crashes: Vec<Instant> = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        let mut child = paper::spawn_primary(settings, dbus_address)?;
        fs::write(settings.paper_pid_file(), child.id().to_string())?;
        log(&format!(
            "started Paper (pid {}) on display {}; waiting for MCP on {}",
            child.id(),
            settings.display,
            settings.mcp_url()
        ));

        let started = Instant::now();
        let mut announced = false;
        let exit_status = loop {
            if stop.load(Ordering::SeqCst) {
                terminate(&mut child, Duration::from_secs(10));
                log("stopped");
                return Ok(());
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if !announced && port_open(settings.mcp_port) {
                announced = true;
                log(&format!(
                    "MCP server listening on {} (DevTools on {})",
                    settings.mcp_url(),
                    settings.cdp_url()
                ));
            } else if !announced && started.elapsed() > MCP_STARTUP_TIMEOUT {
                announced = true;
                log(&format!(
                    "warning: MCP port {} still closed after {}s; see {}",
                    settings.mcp_port,
                    MCP_STARTUP_TIMEOUT.as_secs(),
                    settings.paper_log().display()
                ));
            }
            sleep(Duration::from_millis(500));
        };

        log(&format!("Paper exited with {exit_status}"));
        let now = Instant::now();
        crashes.retain(|at| now.duration_since(*at) < CRASH_WINDOW);
        crashes.push(now);
        if crashes.len() >= MAX_CRASHES_IN_WINDOW {
            return Err(Error::msg(format!(
                "Paper exited {} times within {}s; giving up. Last log lines:\n{}",
                crashes.len(),
                CRASH_WINDOW.as_secs(),
                paths::tail(&settings.paper_log(), 15)
            )));
        }
        sleep(RESTART_DELAY);
    }
    Ok(())
}

/// Start Xvfb unless a server already owns the display, in which case reuse it.
fn start_xvfb(settings: &Settings) -> Result<Option<Child>> {
    if settings.x_socket().exists() && x_display_answers(settings) {
        log(&format!("reusing existing X display {}", settings.display));
        return Ok(None);
    }
    let xvfb = paths::which("Xvfb")
        .ok_or_else(|| Error::msg("Xvfb is not installed (apt install xvfb)"))?;
    let log_file = fs::File::create(settings.xvfb_log())?;
    let mut child = Command::new(xvfb)
        .arg(&settings.display)
        .args(["-screen", "0", &settings.screen, "-nolisten", "tcp"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file.try_clone()?))
        .stderr(Stdio::from(log_file))
        .spawn()
        .map_err(|error| Error::msg(format!("could not start Xvfb: {error}")))?;
    let ready = wait_for(Duration::from_secs(10), || {
        settings.x_socket().exists() || matches!(child.try_wait(), Ok(Some(_)))
    });
    if let Some(status) = child.try_wait()? {
        return Err(Error::msg(format!(
            "Xvfb exited with {status}:\n{}",
            paths::tail(&settings.xvfb_log(), 10)
        )));
    }
    if !ready {
        terminate(&mut child, Duration::from_secs(2));
        return Err(Error::msg(format!(
            "Xvfb did not create {} within 10s",
            settings.x_socket().display()
        )));
    }
    Ok(Some(child))
}

fn x_display_answers(settings: &Settings) -> bool {
    // A stale socket from a crashed server is common; the lock file's pid tells
    // us whether anyone is behind it.
    let lock = format!("/tmp/.X{}-lock", settings.display_number());
    fs::read_to_string(lock)
        .ok()
        .and_then(|contents| contents.trim().parse::<i32>().ok())
        .is_some_and(|pid| kill(Pid::from_raw(pid), None).is_ok())
}

/// Start a private session bus so Electron's keyring lookups have somewhere to
/// go. Optional: Paper runs without it, just noisier.
fn start_dbus(settings: &Settings) -> Result<Option<Child>> {
    let Some(daemon) = paths::which("dbus-daemon") else {
        log("dbus-daemon not found; continuing without a session bus");
        return Ok(None);
    };
    let socket = settings.dbus_socket();
    let _ = fs::remove_file(&socket);
    let mut child = Command::new(daemon)
        .args(["--session", "--nofork", "--nopidfile"])
        .arg(format!("--address=unix:path={}", socket.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Error::msg(format!("could not start dbus-daemon: {error}")))?;
    if !wait_for(Duration::from_secs(5), || socket.exists()) {
        terminate(&mut child, Duration::from_secs(2));
        log("dbus-daemon did not create its socket; continuing without a session bus");
        return Ok(None);
    }
    Ok(Some(child))
}

pub(crate) fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    )
    .is_ok()
}

fn wait_for(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if condition() {
            return true;
        }
        sleep(Duration::from_millis(100));
    }
    false
}

/// SIGTERM, wait up to `grace`, then SIGKILL.
fn terminate(child: &mut Child, grace: Duration) {
    let pid = Pid::from_raw(child.id() as i32);
    let _ = kill(pid, Signal::SIGTERM);
    let started = Instant::now();
    while started.elapsed() < grace {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn log(message: &str) {
    eprintln!("[paper-headless] {message}");
}

# paper-headless

Run [Paper Desktop](https://paper.design) on a headless Linux server so its
local MCP server is available to the agents running there.

Paper's MCP server lives inside the desktop app and only starts once the app is
signed in. On a server there is no screen and no browser, so paper-headless:

- runs Paper under a virtual X display (Xvfb) as a supervised background service,
- relays the one-time browser sign-in to any machine you like,
- registers the MCP endpoint with Claude Code and Codex.

It does not reimplement or proxy any MCP tools. Agents talk to Paper's own
endpoint at `http://127.0.0.1:29979/mcp` directly.

## Install

Debian/Ubuntu, x86_64 or arm64:

```sh
curl -fsSL https://raw.githubusercontent.com/Maddiaa0/paper-headless/main/install.sh | bash
```

The installer asks before each step: the `paper-headless` binary into
`~/.local/bin`, `xvfb` + `dbus-daemon` + `xdg-utils` from the distro repos,
Paper Desktop from Paper's own apt repository (`download.paper.design`), and
finally the background service. Pass `-s -- --yes` to accept everything, or
`--from-source` to build with cargo (Rust nightly `1.98`, pinned by
`rust-toolchain.toml`).

From a checkout: `cargo install --path . --locked`.

## Getting started

```sh
paper-headless install     # systemd user service + Claude Code / Codex config
paper-headless login       # prints a sign-in URL; open it anywhere, paste the result back
paper-headless check       # sign-in and MCP handshake succeed
```

`login` clicks Paper's "Sign in" button for you and captures the URL the app
wanted to open in a browser. Open that URL on your laptop, sign in, and you
land on an "Opening Paper…" page. Copy the `paper://auth/callback?code=…` link
behind its "click here" fallback (or the page's own address) and paste it at
the prompt, or run `paper-headless login-code '<link>'` later. The session is
stored in Paper's profile and survives restarts, so this is a one-time step.

If Claude Code or Codex are installed, `install` runs `claude mcp add` and
`codex mcp add` for a server named `paper`. Any other MCP client can use the
endpoint as Streamable HTTP with no authentication.

## Commands

```text
paper-headless install [--skip-agents]   Install and start the service; configure agents
paper-headless uninstall [--purge]       Remove the service (--purge also deletes the login)
paper-headless start|stop|restart        Control the service
paper-headless status                    Service, processes, ports, sign-in, MCP handshake
paper-headless logs [-n N] [--journal] [-f]
paper-headless login [--no-wait]         Start sign-in, print the URL, wait for the code
paper-headless login-code <link|code>    Finish sign-in
paper-headless check                     Verify sign-in and MCP initialize handshake
paper-headless configure-agents [--remove]
paper-headless doctor                    Required programs and current state
paper-headless serve                     Foreground mode (what the service runs)
```

Global options (also read from the environment, and written into the unit):
`--data-dir` (`PAPER_HEADLESS_DATA_DIR`, default `~/.local/share/paper-headless`),
`--display` (`:99`), `--screen` (`1600x1000x24`), `--cdp-port` (`9222`),
`--mcp-port` (`29979`), `--paper-binary` (`PAPER_DESKTOP_BIN`), and `--system`
to manage a system unit instead of a user unit.

## How it works

`serve` starts Xvfb on the configured display (or reuses one that is already
there), a private D-Bus session bus, and Paper itself with
`--no-sandbox --disable-gpu --user-data-dir=<data>/profile
--remote-debugging-port=<cdp-port>`. Paper is restarted if it exits; five
exits within two minutes stop the service with the log tail. All three are
children of `serve`, so `systemctl stop` tears everything down.

A tiny `xdg-open` shim is placed first on Paper's `PATH`. When Paper asks the
"browser" to open the sign-in URL, the shim appends it to
`<data>/auth-url.txt` instead. `login` triggers that request with a trusted
click over Chrome DevTools, because the sign-in page's button only reacts to
real input events. `login-code` launches a second Paper instance with the
`paper://auth/callback?code=…` deep link; Electron's single-instance lock
forwards it to the running app, which exchanges the code for a session.
Chromium flushes cookies to disk on a timer, so `login-code` waits about half
a minute before returning to make sure a quick restart cannot lose the login.

The Paper session cookie is stored unencrypted in the profile directory (the
app ships with cookie encryption disabled), so keep `<data>` private.

## Seeing the canvas

Agents get images through Paper's own MCP tools (`get_screenshot`, `export`).
For yourself, the simplest view is the same file in a browser at
`app.paper.design`; edits made through MCP sync there live.

To watch the actual server window, install `x11vnc` (the installer offers it),
run it against the virtual display bound to localhost, and tunnel over SSH:

```sh
# on the server
x11vnc -display :99 -localhost -rfbport 5900 -shared -forever -nopw
# on your machine
ssh -L 5900:127.0.0.1:5900 user@server
# then open a VNC client at localhost:5900
```

## Security notes

- Paper's MCP endpoint has no authentication in this mode and only listens on
  loopback; Paper also rejects requests whose `Host` is not `127.0.0.1:29979`
  or `localhost:29979`. A remote agent needs an SSH port-forward that keeps
  that host header.
- The DevTools port is loopback-only too, but any local process can drive the
  app through it. Set `--cdp-port` to something else if 9222 clashes.
- Nothing here talks to paper.design except Paper itself.

## Uninstall

```sh
paper-headless uninstall --purge      # service + profile (login)
paper-headless configure-agents --remove
```

## Development checks

With the pinned Rust toolchain and BTT installed, run:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
btt check
cargo test --locked
bash -n install.sh
```

Write each test's behavior in its `.tree` file first. Trees beside Rust
modules cover their internal behavior. `tests/commands.tree` covers CLI
commands, including service management and diagnostics, and
`tests/installer.tree` covers installer validation. BTT rejects missing,
extra, out-of-order, and uncovered tests.

The automated suite uses temporary profiles, local HTTP/WebSocket fixtures,
and substitute system commands. It does not require Paper, Xvfb, a login,
or a running systemd manager. Browser authentication and persistence across
real Paper restarts require a separate live check.

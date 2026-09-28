# paper-headless

Run [Paper Desktop](https://paper.design) on a headless Linux server, so agents
running there can use Paper's MCP server.

```sh
curl -fsSL https://raw.githubusercontent.com/Maddiaa0/paper-headless/main/install.sh | bash
paper-headless login    # open the printed URL on any machine, paste the link back
paper-headless check    # MCP is ready
```

Claude Code and Codex are configured for you. Any other MCP client can use
`http://127.0.0.1:29979/mcp` (Streamable HTTP, no auth).

> Unofficial. Not affiliated with or endorsed by Paper.

## Install

The installer supports Debian and Ubuntu on x86_64 and arm64. It asks before
each step:

1. installs the `paper-headless` binary into `~/.local/bin`;
2. installs `xvfb`, `dbus-daemon`, and `xdg-utils` from the distro repos;
3. installs Paper Desktop from Paper's apt repository (`download.paper.design`);
4. installs the background service and registers the MCP server as `paper` with Claude Code and Codex.

Pass `-s -- --yes` to accept every step, or `--from-source` to build with cargo.

To install only the binary, use one of these. Then install Xvfb and Paper
Desktop yourself and run `paper-headless install`:

```sh
npm install -g @maddiaa0/paper-headless
brew install maddiaa0/tap/paper-headless
cargo install paper-headless --locked
```

## Signing in

`login` clicks Paper's "Sign in" button and prints the URL Paper wanted to open
in a browser. Open it on your laptop and sign in. You land on an "Opening
Paper…" page. Copy the `paper://auth/callback?code=…` link behind its "click
here" fallback, or the page's own address, and paste it at the prompt. You can
also run `paper-headless login-code '<link>'` later. The login is stored in
Paper's profile and survives restarts, so you only do this once.

## Commands

```text
paper-headless install [--skip-agents]   Install and start the service; configure agents
paper-headless uninstall [--purge]       Remove the service (--purge also deletes the login)
paper-headless start|stop|restart        Control the service
paper-headless status                    Service, processes, ports, sign-in, MCP handshake
paper-headless logs [-n N] [--journal] [-f]
paper-headless login [--no-wait]         Start sign-in, print the URL, wait for the code
paper-headless login-code <link|code>    Finish sign-in
paper-headless check                     MCP initialize handshake
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

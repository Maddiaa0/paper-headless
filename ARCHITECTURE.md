# Architecture

paper-headless is a small CLI: a supervisor that keeps Paper Desktop alive
under a virtual display, and a handful of setup commands around it. It owns no
protocol of its own; the MCP endpoint agents use is Paper's.

## The one flow that explains everything

```
install ──► systemd user unit: ExecStart=paper-headless serve        service.rs
              │
serve ──────► Xvfb :99 ─► dbus-daemon ─► Paper (--remote-debugging-port)   supervisor.rs, paper.rs
              │             PATH = <data>/bin (xdg-open shim) : $PATH
              │
login ──────► DevTools: click "Sign in" ─► Paper calls xdg-open ─► URL      cdp.rs, auth.rs
              │         written to <data>/auth-url.txt, printed to the user
              ▼
login-code ─► second Paper instance with paper://auth/callback?code=…      paper.rs
              ─► single-instance lock hands it to the running app ─► session cookie
              ▼
check ──────► verify renderer sign-in, then POST initialize              auth.rs, mcp.rs
```

## Module map

| Module | Responsibility |
|---|---|
| `cli` | clap types, dispatch |
| `settings` | data dir, display, ports, Paper binary; `Environment=` lines for the unit |
| `supervisor` | `serve`: Xvfb, session bus, Paper restart loop, signal handling |
| `paper` | Paper launch arguments, the `xdg-open` shim, deep-link delivery |
| `cdp` | minimal DevTools client: list pages, evaluate, trusted click |
| `auth` | sign-in state, `login`, `login-code`, code extraction |
| `mcp` | one `initialize` request as a health probe |
| `service` | systemd unit write/enable/start/stop/journal |
| `agents` | `claude mcp add` / `codex mcp add` |
| `doctor` | `doctor` and `status` output |
| `paths` | owner-only file writes, `which`, pid liveness, log tail |

## Invariants

- **No MCP re-implementation.** The crate sends exactly one MCP method
  (`initialize`) and never proxies traffic; agents use Paper's endpoint.
- **Everything is a child of `serve`.** Xvfb, the bus and Paper are spawned
  by the supervisor so a service stop cannot leave strays. An existing X
  display is reused, not owned.
- **Loopback only.** DevTools and MCP bind 127.0.0.1; the crate never
  changes that.
- **Login uses the running instance.** The service runs with DevTools on from
  the start so `login` works against the live instance, and `login-code`
  waits for Chromium's cookie flush before returning.
- **Unsafe is forbidden.** Signals go through `nix` and `signal-hook`.

## Testing

`btt check` verifies the `.tree` specifications against the Rust tests.
`cargo test --locked` exercises parsing, filesystem permissions, process
control, HTTP and WebSocket protocols, service configuration, agent
registration, CLI dispatch, and installer validation. Tests isolate their
state and substitute system commands instead of changing the host service.

The MCP handshake checks transport availability, not authentication: Paper
can answer it while displaying an authentication error. `check` validates
the renderer's sign-in state first. Error pages fail, and background windows
do not count as a signed-in renderer.

Live verification additionally requires Paper and browser sign-in. Run
`paper-headless login`, `paper-headless check`, then restart and repeat the
check. Confirm a read-only MCP file query succeeds after the restart.

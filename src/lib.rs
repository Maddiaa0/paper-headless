//! paper-headless runs Paper Desktop (paper.design) on a headless Linux host
//! under a virtual X display so its local MCP server is reachable by agents.
//!
//! Module map:
//! - [`cli`]: clap definitions and command dispatch
//! - [`settings`]: data directory, display, ports, and the Paper binary
//! - [`supervisor`]: `serve` — Xvfb, a session bus, and a supervised Paper
//! - [`paper`]: Paper launch arguments, the `xdg-open` shim, deep links
//! - [`cdp`]: a minimal Chrome DevTools Protocol client (trusted clicks)
//! - [`mcp`]: a one-request probe of the MCP transport
//! - [`auth`]: the relayed browser sign-in flow
//! - [`service`]: systemd unit management
//! - [`agents`]: registering the MCP endpoint with Claude Code and Codex
//! - [`doctor`]: environment diagnostics

mod agents;
mod auth;
mod cdp;
mod cli;
mod doctor;
mod error;
mod mcp;
mod paper;
mod paths;
mod service;
mod settings;
mod supervisor;
#[cfg(test)]
mod test_support;

pub use error::{Error, Result};

pub fn run() -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Err(Error::Message(
            "paper-headless only supports Linux; on macOS and Windows run Paper Desktop directly"
                .into(),
        ));
    }
    cli::dispatch(cli::Cli::parse_args())
}

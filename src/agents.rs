//! Register the MCP endpoint with the Claude Code and Codex CLIs when they
//! are installed, and print the manual configuration otherwise.

use crate::{Error, Result, paths};
use std::process::Command;

const SERVER_NAME: &str = "paper";

pub(crate) fn configure(mcp_url: &str, remove: bool) -> Result<()> {
    let mut any = false;
    if let Some(claude) = paths::which("claude") {
        any = true;
        let args: Vec<&str> = if remove {
            vec!["mcp", "remove", "--scope", "user", SERVER_NAME]
        } else {
            vec![
                "mcp",
                "add",
                "--transport",
                "http",
                "--scope",
                "user",
                SERVER_NAME,
                mcp_url,
            ]
        };
        run("claude", &claude, &args)?;
    }
    if let Some(codex) = paths::which("codex") {
        any = true;
        let args: Vec<&str> = if remove {
            vec!["mcp", "remove", SERVER_NAME]
        } else {
            vec!["mcp", "add", SERVER_NAME, "--url", mcp_url]
        };
        run("codex", &codex, &args)?;
    }
    if !any && !remove {
        println!("neither `claude` nor `codex` is on PATH; configure them by hand:");
        println!();
        println!("  claude mcp add --transport http --scope user {SERVER_NAME} {mcp_url}");
        println!("  codex mcp add {SERVER_NAME} --url {mcp_url}");
        println!();
        println!("Any other MCP client: Streamable HTTP at {mcp_url} (no auth, localhost only).");
    }
    Ok(())
}

fn run(name: &str, binary: &std::path::Path, args: &[&str]) -> Result<()> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .map_err(|error| Error::msg(format!("could not run {name}: {error}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if output.status.success() {
        println!("{name}: {}", if stdout.is_empty() { "ok" } else { &stdout });
        Ok(())
    } else {
        let already = stderr.contains("already exists") || stdout.contains("already exists");
        if already {
            println!("{name}: server `{SERVER_NAME}` already configured");
            return Ok(());
        }
        Err(Error::msg(format!(
            "{name} {} failed: {}",
            args.join(" "),
            if stderr.is_empty() { stdout } else { stderr }
        )))
    }
}

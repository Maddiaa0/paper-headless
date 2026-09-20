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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Temp;
    mod when_running_an_agent_command {
        use super::*;
        #[test]
        fn accepts_successful_registration() {
            let t = Temp::new();
            let p = t.script("agent", "echo registered");
            run("agent", &p, &["mcp", "add"]).unwrap();
        }
        #[test]
        fn accepts_an_existing_server() {
            let t = Temp::new();
            for body in [
                "echo already exists >&2; exit 1",
                "echo already exists; exit 1",
            ] {
                let p = t.script("agent", body);
                assert!(run("agent", &p, &[]).is_ok());
            }
        }
        #[test]
        fn reports_stderr_failures() {
            let t = Temp::new();
            let p = t.script("agent", "echo denied >&2; exit 1");
            assert!(
                run("agent", &p, &["mcp"])
                    .unwrap_err()
                    .to_string()
                    .contains("denied")
            );
        }
        #[test]
        fn reports_stdout_failures() {
            let t = Temp::new();
            let p = t.script("agent", "echo failed; exit 1");
            assert!(
                run("agent", &p, &[])
                    .unwrap_err()
                    .to_string()
                    .contains("failed")
            );
        }
        #[test]
        fn reports_spawn_failures() {
            let t = Temp::new();
            assert!(
                run("agent", &t.path.join("missing"), &[])
                    .unwrap_err()
                    .to_string()
                    .contains("could not run agent")
            );
        }
    }
}

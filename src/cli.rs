//! Command-line surface. Global flags (data dir, display, ports, Paper binary)
//! double as environment variables so the systemd unit reproduces them.

use crate::service::Scope;
use crate::settings::{
    DEFAULT_CDP_PORT, DEFAULT_DISPLAY, DEFAULT_MCP_PORT, DEFAULT_SCREEN, Settings,
};
use crate::{Result, agents, auth, doctor, mcp, paths, service, supervisor};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "paper-headless",
    version,
    about = "Run Paper Desktop on a headless Linux server so its MCP server is available to agents"
)]
pub(crate) struct Cli {
    #[command(flatten)]
    global: GlobalArgs,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args)]
struct GlobalArgs {
    /// Where the Paper profile, logs, and helper files live
    #[arg(
        long,
        global = true,
        env = "PAPER_HEADLESS_DATA_DIR",
        value_name = "DIR"
    )]
    data_dir: Option<PathBuf>,
    /// X display to create with Xvfb (or reuse if it already exists)
    #[arg(long, global = true, env = "PAPER_HEADLESS_DISPLAY", default_value = DEFAULT_DISPLAY)]
    display: String,
    /// Xvfb screen geometry
    #[arg(long, global = true, env = "PAPER_HEADLESS_SCREEN", default_value = DEFAULT_SCREEN)]
    screen: String,
    /// Loopback port for Paper's DevTools (used only to click "Sign in")
    #[arg(long, global = true, env = "PAPER_HEADLESS_CDP_PORT", default_value_t = DEFAULT_CDP_PORT)]
    cdp_port: u16,
    /// Port Paper's MCP server listens on (fixed by Paper; change only if Paper does)
    #[arg(long, global = true, env = "PAPER_HEADLESS_MCP_PORT", default_value_t = DEFAULT_MCP_PORT)]
    mcp_port: u16,
    /// Path to the Paper Desktop executable
    #[arg(long, global = true, env = "PAPER_DESKTOP_BIN", value_name = "PATH")]
    paper_binary: Option<PathBuf>,
    /// Manage a systemd system unit instead of a user unit
    #[arg(long, global = true)]
    system: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Run Xvfb and Paper in the foreground (what the service runs)
    Serve,
    /// Install and start the systemd service, then register the MCP endpoint with Claude Code and Codex
    Install {
        /// Do not touch Claude Code or Codex configuration
        #[arg(long)]
        skip_agents: bool,
    },
    /// Stop and remove the systemd service
    Uninstall {
        /// Also delete the data directory, including the Paper login
        #[arg(long)]
        purge: bool,
    },
    /// Start the service
    Start,
    /// Stop the service
    Stop,
    /// Restart the service
    Restart,
    /// Show service, process, port, and sign-in state
    Status,
    /// Show recent Paper output (or the service journal with --journal)
    Logs {
        /// Number of lines
        #[arg(short = 'n', long, default_value_t = 60)]
        lines: usize,
        /// Read the systemd journal for the service instead of Paper's log file
        #[arg(long)]
        journal: bool,
        /// Follow the journal (implies --journal)
        #[arg(short, long)]
        follow: bool,
    },
    /// Start the browser sign-in and print the URL to open elsewhere
    Login {
        /// Print the URL and exit instead of waiting for the pasted code
        #[arg(long)]
        no_wait: bool,
    },
    /// Finish sign-in with the paper://auth/callback link (or bare code) from the browser
    LoginCode {
        /// The link or code shown after signing in
        value: String,
        /// Do not wait for Chromium to persist the session before returning
        #[arg(long)]
        no_wait: bool,
    },
    /// Verify the MCP endpoint accepts an initialize handshake
    Check,
    /// Register (or, with --remove, unregister) the MCP endpoint with Claude Code and Codex
    ConfigureAgents {
        #[arg(long)]
        remove: bool,
    },
    /// Check required programs and current state
    Doctor,
}

impl Cli {
    pub(crate) fn parse_args() -> Self {
        Self::parse()
    }
}

pub(crate) fn dispatch(cli: Cli) -> Result<()> {
    let g = cli.global;
    let settings = Settings::resolve(
        g.data_dir,
        g.display,
        g.screen,
        g.cdp_port,
        g.mcp_port,
        g.paper_binary,
    )?;
    let scope = Scope::from_flag(g.system);
    match cli.command {
        Commands::Serve => supervisor::serve(&settings),
        Commands::Install { skip_agents } => {
            service::install(&settings, scope)?;
            if !skip_agents {
                agents::configure(&settings.mcp_url(), false)?;
            }
            println!();
            println!("Next: paper-headless login");
            Ok(())
        }
        Commands::Uninstall { purge } => service::uninstall(&settings, scope, purge),
        Commands::Start => service::start(scope),
        Commands::Stop => service::stop(scope),
        Commands::Restart => service::restart(scope),
        Commands::Status => doctor::status(&settings, scope),
        Commands::Logs {
            lines,
            journal,
            follow,
        } => {
            if journal || follow {
                service::journal(scope, lines, follow)
            } else {
                let log = settings.paper_log();
                let text = paths::tail(&log, lines);
                if text.is_empty() {
                    println!("(no output yet in {})", log.display());
                } else {
                    println!("{text}");
                }
                Ok(())
            }
        }
        Commands::Login { no_wait } => auth::login(&settings, !no_wait),
        Commands::LoginCode { value, no_wait } => auth::login_code(&settings, &value, !no_wait),
        Commands::Check => {
            let server = mcp::require_ready(&settings.mcp_url())?;
            println!("MCP ready at {} ({server})", settings.mcp_url());
            Ok(())
        }
        Commands::ConfigureAgents { remove } => agents::configure(&settings.mcp_url(), remove),
        Commands::Doctor => doctor::doctor(&settings, scope),
    }
}

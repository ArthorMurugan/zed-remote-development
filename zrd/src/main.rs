//! zrd — the native agent for Zed Remote Development.
//!
//! zrd connects to machines over SSH, discovers development environments
//! (Docker / Dev Containers / Compose), manages their lifecycle, resolves
//! workspaces, and hands off to Zed's native remote-development via the
//! documented `zed ssh://...` CLI.

mod activity;
mod compose;
mod config;
mod devcontainer;
mod diagnostics;
mod discovery;
mod docker;
mod environment;
mod models;
mod server;
mod ssh;
mod workspace;
mod zed_launcher;

use anyhow::Result;
use clap::{Parser, Subcommand};
use zed_launcher::ZedLauncher;

#[derive(Parser)]
#[command(
    name = "zrd",
    version,
    about = "Zed Remote Development agent — connect, discover, develop."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List machines (registered + ~/.ssh/config hosts).
    Machines {
        /// Output machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add a machine. The simplest form is just an address: zrd add-machine user@host
    AddMachine {
        /// user@host, host, or user@host:port
        address: String,
        /// Friendly name shown in pickers.
        #[arg(long)]
        name: Option<String>,
        /// SSH port (advanced).
        #[arg(long)]
        port: Option<u16>,
        /// Path to an identity file (advanced).
        #[arg(long)]
        identity: Option<String>,
        /// Proxy/jump host (advanced).
        #[arg(long)]
        jump: Option<String>,
    },
    /// Verify a connection to a machine.
    Connect {
        machine: String,
        #[arg(long)]
        json: bool,
    },
    /// Discover development environments and projects on a machine.
    Discover {
        machine: String,
        #[arg(long)]
        json: bool,
    },
    /// Connect, start the environment if needed, and open the project in Zed.
    Open {
        machine: String,
        /// Environment to open (auto-selected when there is only one).
        #[arg(long)]
        env: Option<String>,
        /// Project path to open.
        #[arg(long)]
        project: Option<String>,
        /// Do everything except launching Zed.
        #[arg(long)]
        no_launch: bool,
        #[arg(long)]
        json: bool,
    },
    /// Reopen the most recent development session.
    Reopen {
        #[arg(long)]
        env: Option<String>,
        #[arg(long)]
        no_launch: bool,
        #[arg(long)]
        json: bool,
    },
    /// Rebuild an environment (devcontainer rebuild / compose --build).
    Rebuild {
        machine: String,
        #[arg(long)]
        env: String,
    },
    /// Show recent logs for an environment.
    Logs {
        machine: String,
        #[arg(long)]
        env: String,
        #[arg(long, default_value = "100")]
        tail: u32,
    },
    /// Show recent sessions.
    Recent {
        #[arg(long)]
        json: bool,
    },
    /// Run infrastructure diagnostics for a machine.
    Diagnose {
        machine: String,
        #[arg(long)]
        json: bool,
    },
    /// Open a remote workspace in Zed via the documented ssh:// CLI URL.
    Launch {
        /// ssh://[user@]host[:port]/path
        url: String,
    },
    /// Run the stdio JSON-lines server used by the Zed extension.
    Serve,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Machines { json } => {
            let machines = server::list_machines()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&machines)?);
            } else if machines.is_empty() {
                println!("No machines yet. Add one with:\n\n  zrd add-machine user@server.example.com\n");
            } else {
                println!("Machines\n");
                for m in &machines {
                    println!("  {}  ({})", m.name, m.target);
                }
                println!("\n  + Add Machine: zrd add-machine user@host");
            }
        }
        Commands::AddMachine {
            address,
            name,
            port,
            identity,
            jump,
        } => {
            let mut target = ssh::SshTarget::parse(&address)?;
            target.port = port.or(target.port);
            if let Some(identity) = identity {
                target.extra_args.extend(["-i".into(), identity]);
            }
            if let Some(jump) = jump {
                target.extra_args.extend(["-J".into(), jump]);
            }
            let name = name.unwrap_or_else(|| target.authority());
            let mut registry = config::MachineRegistry::load()?;
            registry.add(name.clone(), target);
            registry.save()?;
            println!("Added machine '{name}'.");
        }
        Commands::Connect { machine, json } => {
            let target = server::resolve_target(&machine)?;
            let mut progress = Vec::new();
            server::connect(&target, &mut progress)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "connected": true,
                        "progress": progress,
                    }))?
                );
            } else {
                for line in &progress {
                    println!("✓ {line}");
                }
            }
        }
        Commands::Discover { machine, json } => {
            let target = server::resolve_target(&machine)?;
            let d = server::discover(&target, None)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&d)?);
            } else {
                print_discovery(&d);
            }
        }
        Commands::Open {
            machine,
            env,
            project,
            no_launch,
            json,
        } => {
            let target = server::resolve_target(&machine)?;
            let result = server::open(&server::OpenRequest {
                target,
                environment: env,
                project,
                no_launch,
            })?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for line in &result.progress {
                    println!("✓ {line}");
                }
                println!("\nReady: {}", result.zed_url);
            }
        }
        Commands::Reopen {
            env,
            no_launch,
            json,
        } => {
            let result = server::reopen(env.as_deref(), no_launch)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for line in &result.progress {
                    println!("✓ {line}");
                }
                println!("\nReady: {}", result.zed_url);
            }
        }
        Commands::Rebuild { machine, env } => {
            let target = server::resolve_target(&machine)?;
            for line in server::rebuild(&target, &env)? {
                println!("✓ {line}");
            }
        }
        Commands::Logs { machine, env, tail } => {
            let target = server::resolve_target(&machine)?;
            print!("{}", server::logs(&target, &env, tail)?);
        }
        Commands::Recent { json } => {
            let state = config::State::load()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&state.recent)?);
            } else if state.recent.is_empty() {
                println!("No recent development sessions.");
            } else {
                println!("Recent Development\n");
                for s in &state.recent {
                    let label = s
                        .environment
                        .clone()
                        .or(s.activity.clone())
                        .unwrap_or_else(|| s.workspace.clone());
                    println!("  ★ {}  —  {}", label, s.machine.display_name());
                }
            }
        }
        Commands::Diagnose { machine, json } => {
            let target = server::resolve_target(&machine)?;
            let shell = ssh::RemoteShell::new(target);
            let diag = diagnostics::collect(&shell);
            if json {
                println!("{}", serde_json::to_string_pretty(&diag)?);
            } else {
                print!("{}", diagnostics::render_human(&diag));
            }
        }
        Commands::Serve => server::serve()?,
        Commands::Launch { url } => {
            // Parse the URL back into a target + workspace and hand off
            // through the ZedLauncher abstraction.
            let (authority, path) = url
                .strip_prefix("ssh://")
                .and_then(|rest| rest.split_once('/'))
                .map(|(a, p)| (a.to_string(), format!("/{p}")))
                .ok_or_else(|| anyhow::anyhow!("invalid ssh:// URL: {url}"))?;
            let target = ssh::SshTarget::parse(&authority)?;
            let connection = zed_launcher::RemoteConnection { target };
            zed_launcher::CliZedLauncher::default()
                .open_remote_project(&connection, &path)?;
            println!("Opening {url} in Zed");
        }
    }
    Ok(())
}

fn print_discovery(d: &models::Discovery) {
    for line in &d.progress {
        println!("✓ {line}");
    }
    if !d.activities.is_empty() {
        println!("\nDevelopment Activities\n");
        for a in &d.activities {
            let detail = [
                a.stack.clone().unwrap_or_default(),
                a.environment.clone().unwrap_or_default(),
                a.workspace.clone(),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            println!("  ▶ {}", a.name);
            println!("    {detail}");
        }
    }
    if !d.environments.is_empty() {
        println!("\nEnvironments\n");
        for e in &d.environments {
            let detail = e.description.clone().unwrap_or_default();
            println!(
                "  {}  {}  ({}){}",
                match e.kind {
                    models::EnvironmentKind::Docker => "🐳",
                    models::EnvironmentKind::DevContainer => "📦",
                    models::EnvironmentKind::Compose => "🧩",
                },
                e.name,
                e.state,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!("  {detail}")
                }
            );
        }
    }
    if !d.projects.is_empty() {
        println!("\nProjects\n");
        for p in &d.projects {
            println!("  {}  ({})", p.path, p.markers.join(", "));
        }
    }
}

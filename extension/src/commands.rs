//! Slash-command handlers.
//!
//! Zed extensions cannot register command-palette commands or UI pickers
//! (see ASSUMPTIONS.md), so the VS Code-style commands are mapped onto
//! slash commands with argument completion standing in for pickers:
//!
//!   Remote Development: Connect              -> /remote-connect
//!   Remote Development: Open Environment     -> /remote-open
//!   Remote Development: Reopen in Environment-> /remote-reopen
//!   Remote Development: Rebuild Environment  -> /remote-open ... --rebuild (via zrd)
//!   Remote Development: Show Logs            -> /remote-logs
//!   (diagnostics)                            -> /remote-diagnose

use crate::client::ZrdClient;
use crate::models::{Discovery, Environment, EnvironmentKind};
use crate::zed_api::{RemoteTarget, WasmZedHost, ZedHost};
use zed_extension_api as zed;

fn sections(text: &str) -> Vec<zed::SlashCommandOutputSection> {
    if text.is_empty() {
        return Vec::new();
    }
    let len = text.len();
    vec![zed::SlashCommandOutputSection {
        range: (0..len).into(),
        label: String::new(),
    }]
}

fn output(text: String) -> Result<zed::SlashCommandOutput, String> {
    let sections = sections(&text);
    Ok(zed::SlashCommandOutput { text, sections })
}

fn kind_icon(kind: EnvironmentKind) -> &'static str {
    match kind {
        EnvironmentKind::Docker => "🐳",
        EnvironmentKind::DevContainer => "📦",
        EnvironmentKind::Compose => "🧩",
    }
}

fn describe_environment(env: &Environment) -> String {
    let mut line = format!(
        "{} {}  —  {}",
        kind_icon(env.kind),
        env.name,
        env.state
    );
    if let Some(desc) = &env.description {
        line.push_str(&format!("  ({desc})"));
    }
    line
}

/// `/remote-connect [machine]`
///
/// No argument: list machines. With a machine: connect and discover its
/// development environments, showing the user exactly what to run next.
pub fn connect(client: &ZrdClient, args: &[String]) -> Result<zed::SlashCommandOutput, String> {
    let Some(machine) = args.first() else {
        let machines = client.machines()?;
        let mut text = String::from("Remote Development\n\nMachines\n\n");
        if machines.is_empty() {
            text.push_str("  No machines yet.\n\n  Add one: /remote-add user@server.example.com\n");
        } else {
            for (i, m) in machines.iter().enumerate() {
                text.push_str(&format!("  {}. {}\n", i + 1, m.name));
            }
            text.push_str("\n  + Add Machine: /remote-add user@host\n");
            text.push_str("\nConnect with: /remote-connect <machine>\n");
        }
        return output(text);
    };

    let discovery = client.discover(machine)?;
    let mut text = format!("{machine}\n\n");
    for line in &discovery.progress {
        text.push_str(&format!("✓ {line}\n"));
    }
    text.push_str(&environment_overview(&discovery));
    text.push_str(&format!(
        "\nOpen with: /remote-open {machine} <environment>\n"
    ));
    output(text)
}

fn environment_overview(d: &Discovery) -> String {
    let mut text = String::new();
    if !d.activities.is_empty() {
        text.push_str("\nDevelopment Activities\n\n");
        for a in &d.activities {
            text.push_str(&format!("▶ {}\n", a.name));
            let detail = [
                a.stack.clone().unwrap_or_default(),
                a.environment.clone().unwrap_or_default(),
                a.workspace.clone(),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            text.push_str(&format!("  {detail}\n"));
        }
    }
    if !d.environments.is_empty() {
        text.push_str("\nEnvironments\n\n");
        for env in &d.environments {
            text.push_str(&describe_environment(env));
            text.push('\n');
        }
    }
    if d.environments.is_empty() && !d.projects.is_empty() {
        text.push_str("\nProjects (no container environments — open directly)\n\n");
        for p in &d.projects {
            text.push_str(&format!("  {}\n", p.path));
        }
    }
    text
}

/// `/remote-open <machine> [environment]`
///
/// The MVP vertical slice: connect → discover → select → start if needed →
/// resolve workspace → open Zed on the project.
pub fn open(client: &ZrdClient, args: &[String]) -> Result<zed::SlashCommandOutput, String> {
    let Some(machine) = args.first() else {
        return Err("Usage: /remote-open <machine> [environment]".into());
    };
    let environment = args.get(1).map(|s| s.as_str());
    // Orchestration happens in zrd; the actual hand-off to Zed goes through
    // the ZedHost abstraction so the extension never bakes in CLI details.
    let host = WasmZedHost;
    let result = client.open_opts(machine, environment, true)?;

    let mut text = String::new();
    for line in &result.progress {
        text.push_str(&format!("✓ {line}\n"));
    }
    host.show_message(&format!("Opening {} in Zed", result.workspace));
    host.open_remote_project(RemoteTarget {
        ssh_url: result.zed_url.clone(),
    })?;
    text.push_str("✓ Opening Zed...\n");
    if let Some(env) = &result.environment {
        text.push_str(&format!(
            "\nOpening {} ({})\n  workspace: {}\n",
            env.name, env.state, result.workspace
        ));
    } else {
        text.push_str(&format!("\nOpening {}\n", result.workspace));
    }
    text.push_str(&format!("\nZed URL: {}\n", result.zed_url));
    output(text)
}

/// `/remote-reopen [environment]` — reconnect to the most recent session.
pub fn reopen(client: &ZrdClient, args: &[String]) -> Result<zed::SlashCommandOutput, String> {
    // With no argument and no recorded sessions, explain instead of failing.
    if args.is_empty() {
        if let Ok(recent) = client.recent() {
            if recent.is_empty() {
                return output(
                    "No recent development sessions.\n\nStart one with /remote-connect".into(),
                );
            }
        }
    }
    let environment = args.first().map(|s| s.as_str());
    let result = client.reopen_opts(environment, true)?;
    let mut text = String::from("Reconnecting...\n\n");
    for line in &result.progress {
        text.push_str(&format!("✓ {line}\n"));
    }
    let host = WasmZedHost;
    host.open_remote_project(RemoteTarget {
        ssh_url: result.zed_url.clone(),
    })?;
    text.push_str("✓ Opening Zed...\n");
    text.push_str(&format!("\nZed URL: {}\n", result.zed_url));
    output(text)
}

/// `/remote-add <user@host>`
pub fn add(client: &ZrdClient, args: &[String]) -> Result<zed::SlashCommandOutput, String> {
    let Some(address) = args.first() else {
        return Err(
            "Usage: /remote-add user@server.example.com\n\n\
             Your existing ~/.ssh/config is used automatically; advanced \
             options (port, identity, jump host) can go there or in \
             `zrd add-machine` flags."
                .into(),
        );
    };
    client.add_machine(address)?;
    output(format!(
        "Added machine '{address}'.\n\nConnect with: /remote-connect {address}"
    ))
}

/// `/remote-logs <machine> <environment>`
pub fn logs(client: &ZrdClient, args: &[String]) -> Result<zed::SlashCommandOutput, String> {
    let (Some(machine), Some(env)) = (args.first(), args.get(1)) else {
        return Err("Usage: /remote-logs <machine> <environment>".into());
    };
    let logs = client.logs(machine, env)?;
    output(format!("Logs: {env} on {machine}\n\n{logs}"))
}

/// `/remote-diagnose <machine>`
pub fn diagnose(client: &ZrdClient, args: &[String]) -> Result<zed::SlashCommandOutput, String> {
    let Some(machine) = args.first() else {
        return Err("Usage: /remote-diagnose <machine>".into());
    };
    output(client.diagnose(machine)?)
}

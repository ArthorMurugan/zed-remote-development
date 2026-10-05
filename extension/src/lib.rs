//! Zed Remote Development extension.
//!
//! A thin WASM front-end for the `zrd` native agent. The extension owns
//! user interaction (slash commands + argument completion, the only UI
//! primitives Zed currently offers extensions); zrd owns SSH, Docker, dev
//! containers, compose, lifecycle, and the Zed CLI hand-off.

mod client;
mod commands;
mod models;
mod zed_api;

use client::ZrdClient;
use zed_extension_api as zed;

struct ZedRemoteDevelopment;

impl ZedRemoteDevelopment {
    fn client(&self) -> ZrdClient {
        ZrdClient::default()
    }

    /// Argument completions double as pickers: each candidate is an
    /// actionable choice, and accepting one runs the command.
    fn complete(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<Vec<zed::SlashCommandArgumentCompletion>, String> {
        let client = self.client();
        match (command, args.len()) {
            // /remote-connect <machine>
            ("remote-connect", _) | ("remote-diagnose", _) => {
                let machines = client.machines()?;
                Ok(machines
                    .into_iter()
                    .map(|m| zed::SlashCommandArgumentCompletion {
                        label: format!("{} ({})", m.name, m.source),
                        new_text: m.name,
                        run_command: command == "remote-connect",
                    })
                    .collect())
            }
            // /remote-open <machine> [environment]
            ("remote-open", 0) | ("remote-logs", 0) => {
                let machines = client.machines()?;
                Ok(machines
                    .into_iter()
                    .map(|m| zed::SlashCommandArgumentCompletion {
                        label: m.name.clone(),
                        new_text: m.name,
                        run_command: false,
                    })
                    .collect())
            }
            ("remote-open", _) | ("remote-logs", _) => {
                let machine = &args[0];
                let discovery = client.discover(machine)?;
                Ok(discovery
                    .environments
                    .into_iter()
                    .map(|e| zed::SlashCommandArgumentCompletion {
                        label: format!("{} — {}", e.name, e.state),
                        new_text: format!("{machine} {}", e.name),
                        run_command: true,
                    })
                    .collect())
            }
            // /remote-reopen [environment] — offer recent sessions.
            ("remote-reopen", _) => {
                let recent = client.recent()?;
                Ok(recent
                    .into_iter()
                    .filter_map(|s| {
                        s.environment.map(|env| zed::SlashCommandArgumentCompletion {
                            label: format!("★ {env} — {}", s.workspace),
                            new_text: env,
                            run_command: true,
                        })
                    })
                    .collect())
            }
            _ => Ok(Vec::new()),
        }
    }
}

impl zed::Extension for ZedRemoteDevelopment {
    fn new() -> Self {
        ZedRemoteDevelopment
    }

    fn complete_slash_command_argument(
        &self,
        command: zed::SlashCommand,
        args: Vec<String>,
    ) -> Result<Vec<zed::SlashCommandArgumentCompletion>, String> {
        self.complete(&command.name, &args)
    }

    fn run_slash_command(
        &self,
        command: zed::SlashCommand,
        args: Vec<String>,
        _worktree: Option<&zed::Worktree>,
    ) -> Result<zed::SlashCommandOutput, String> {
        let client = self.client();
        match command.name.as_str() {
            "remote-connect" => commands::connect(&client, &args),
            "remote-open" => commands::open(&client, &args),
            "remote-reopen" => commands::reopen(&client, &args),
            "remote-add" => commands::add(&client, &args),
            "remote-logs" => commands::logs(&client, &args),
            "remote-diagnose" => commands::diagnose(&client, &args),
            other => Err(format!("unknown command: {other}")),
        }
    }
}

zed::register_extension!(ZedRemoteDevelopment);

//! Core operations and the stdio server the Zed extension talks to.
//!
//! The extension is a WASM sandbox; zrd is the native process that does the
//! real work. Two transports reach the same operations:
//!   - one-shot CLI commands (`zrd open ...`)
//!   - a long-lived JSON-lines server (`zrd serve`) reading requests on
//!     stdin and writing responses on stdout

use crate::config::{MachineRegistry, State};
use crate::discovery;
use crate::environment;
use crate::models::{Discovery, Machine, OpenResult, RecentSession};
use crate::ssh::{RemoteShell, SshConfig, SshTarget};
use crate::workspace;
use crate::zed_launcher::{zed_ssh_url, CliZedLauncher, RemoteConnection, ZedLauncher};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

/// All machines known to zrd: registered machines first, then hosts from
/// `~/.ssh/config` (the user's existing SSH config is the source of truth).
pub fn list_machines() -> Result<Vec<Machine>> {
    let mut machines = Vec::new();
    for m in MachineRegistry::load()?.machines {
        machines.push(Machine {
            name: m.name,
            target: m.target,
            source: "zrd-config".into(),
        });
    }
    if let Ok(cfg) = SshConfig::load() {
        for host in &cfg.hosts {
            let target = SshConfig::to_target(host);
            // Don't duplicate a registered machine that aliases the same host.
            if machines
                .iter()
                .any(|m| m.name == host.pattern || m.target.host == target.host)
            {
                continue;
            }
            machines.push(Machine {
                name: host.pattern.clone(),
                target,
                source: "ssh-config".into(),
            });
        }
    }
    Ok(machines)
}

/// Resolve a machine name or address into a concrete target.
pub fn resolve_target(name_or_address: &str) -> Result<SshTarget> {
    if let Some(t) = MachineRegistry::load()?.find(name_or_address) {
        return Ok(t);
    }
    // Try the SSH config aliases too.
    if let Ok(cfg) = SshConfig::load() {
        if let Some(host) = cfg.hosts.iter().find(|h| h.pattern == name_or_address) {
            return Ok(SshConfig::to_target(host));
        }
    }
    SshTarget::parse(name_or_address)
        .with_context(|| format!("unknown machine '{name_or_address}'"))
}

/// Connect to a machine: verify SSH reachability and report progress.
pub fn connect(target: &SshTarget, progress: &mut Vec<String>) -> Result<RemoteShell> {
    progress.push(format!("Connecting to {}...", target.display_name()));
    let shell = RemoteShell::new(target.clone());
    let uname = shell.check_connection().context(format!(
        "could not connect to {}. Check that the machine is reachable and your SSH setup works.",
        target.display_name()
    ))?;
    progress.push("Connected".into());
    progress.push(format!("Remote: {uname}"));
    Ok(shell)
}

/// Discover environments, projects and activities on a machine.
pub fn discover(target: &SshTarget, roots: Option<Vec<String>>) -> Result<Discovery> {
    let mut progress = Vec::new();
    let shell = connect(target, &mut progress)?;
    let mut d = discovery::discover(&shell, roots)?;
    d.progress = progress.into_iter().chain(d.progress).collect();
    Ok(d)
}

pub struct OpenRequest {
    pub target: SshTarget,
    /// Specific environment name, if the user picked one.
    pub environment: Option<String>,
    /// Specific project path, if the user picked one.
    pub project: Option<String>,
    /// Skip launching Zed (used by tests and dry runs).
    pub no_launch: bool,
}

/// The full flow: connect → discover → select environment → ensure running
/// → resolve workspace → launch Zed → remember the session.
pub fn open(req: &OpenRequest) -> Result<OpenResult> {
    let mut progress = Vec::new();
    let shell = connect(&req.target, &mut progress)?;
    let discovery = discovery::discover(&shell, None)?;
    progress.extend(discovery.progress.clone());

    // --- Environment selection -------------------------------------------
    // The simplest path is the default: zero environments means "open the
    // remote folder directly"; exactly one means "just use it"; several
    // means the caller (extension picker or user) must choose.
    let mut chosen: Option<crate::models::Environment> = match &req.environment {
        Some(name) => Some(
            discovery
                .environments
                .iter()
                .find(|e| &e.name == name)
                .cloned()
                .ok_or_else(|| {
                    let known = discovery
                        .environments
                        .iter()
                        .map(|e| e.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    anyhow!("no environment named '{name}' on this machine (found: {known})")
                })?,
        ),
        None if discovery.environments.len() == 1 => {
            let only = discovery.environments[0].clone();
            progress.push(format!(
                "Using the only environment: {}",
                only.name
            ));
            Some(only)
        }
        None if discovery.environments.is_empty() => None,
        None => {
            let names = discovery
                .environments
                .iter()
                .map(|e| e.name.clone())
                .collect::<Vec<_>>();
            return Err(anyhow!(
                "multiple development environments found — please choose one of: {}",
                names.join(", ")
            ));
        }
    };

    // --- Lifecycle ---------------------------------------------------------
    if let Some(env) = chosen.as_mut() {
        environment::ensure_running(&shell, env, &mut progress).map_err(|e| {
            anyhow!(
                "Unable to start {}.\n\nThe development environment failed to start. \
                 Run `zrd logs {} --env {}` or `zrd diagnose {}` for details.\n\n({e:#})",
                env.name,
                req.target.display_name(),
                env.name,
                req.target.display_name()
            )
        })?;
    }

    // --- Workspace resolution ----------------------------------------------
    let workspace = resolve_workspace(req, chosen.as_ref(), &discovery)?;
    progress.push(format!("Workspace: {workspace}"));

    // --- Hand-off ----------------------------------------------------------
    let connection = RemoteConnection {
        target: req.target.clone(),
    };
    let url = zed_ssh_url(&req.target, &workspace);
    if !req.no_launch {
        progress.push("Opening Zed...".into());
        CliZedLauncher::default().open_remote_project(&connection, &workspace)?;
        progress.push("Zed is opening the project".into());
    }

    // --- Remember for one-click reconnect -----------------------------------
    let mut state = State::load().unwrap_or_default();
    state.remember(RecentSession {
        machine: req.target.clone(),
        environment: chosen.as_ref().map(|e| e.name.clone()),
        workspace: workspace.clone(),
        activity: None,
        last_opened_at: now_iso8601(),
    });
    let _ = state.save();

    Ok(OpenResult {
        machine: req.target.clone(),
        environment: chosen,
        workspace,
        zed_url: url,
        progress,
    })
}

fn resolve_workspace(
    req: &OpenRequest,
    env: Option<&crate::models::Environment>,
    discovery: &Discovery,
) -> Result<String> {
    if let Some(project) = &req.project {
        return Ok(project.clone());
    }
    if let Some(env) = env {
        if let Some(ws) = env.workspace.clone().or_else(|| env.project_path.clone()) {
            return Ok(ws);
        }
    }
    if discovery.projects.len() == 1 {
        return Ok(discovery.projects[0].path.clone());
    }
    // Fall back to the machine's home directory; Zed's remote file picker
    // takes it from there.
    Ok("~".into())
}

/// Reopen the most recent session (or the one named by `environment`).
pub fn reopen(environment: Option<&str>, no_launch: bool) -> Result<OpenResult> {
    let state = State::load()?;
    let session = match environment {
        Some(name) => state
            .recent
            .iter()
            .find(|s| s.environment.as_deref() == Some(name) || s.activity.as_deref() == Some(name))
            .cloned(),
        None => state.recent.first().cloned(),
    }
    .ok_or_else(|| anyhow!("no recent development session to reopen"))?;

    open(&OpenRequest {
        target: session.machine,
        environment: session.environment,
        project: Some(session.workspace),
        no_launch,
    })
}

/// Rebuild an environment on a machine.
pub fn rebuild(target: &SshTarget, env_name: &str) -> Result<Vec<String>> {
    let mut progress = Vec::new();
    let shell = connect(target, &mut progress)?;
    let d = discovery::discover(&shell, None)?;
    let env = d
        .environments
        .iter()
        .find(|e| e.name == env_name)
        .ok_or_else(|| anyhow!("no environment named '{env_name}' on this machine"))?;
    environment::rebuild(&shell, env, &mut progress)?;
    Ok(progress)
}

/// Fetch recent logs for an environment.
pub fn logs(target: &SshTarget, env_name: &str, tail: u32) -> Result<String> {
    let shell = RemoteShell::new(target.clone());
    let d = discovery::discover(&shell, None)?;
    let env = d
        .environments
        .iter()
        .find(|e| e.name == env_name)
        .ok_or_else(|| anyhow!("no environment named '{env_name}' on this machine"))?;
    environment::logs(&shell, env, tail)
}

fn now_iso8601() -> String {
    // Avoid pulling in a time crate: seconds since epoch are unambiguous
    // enough for ordering recents, and formatted where displayed.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

// ---------------------------------------------------------------------------
// stdio JSON-lines server (`zrd serve`)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Request {
    id: u64,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct Response {
    id: u64,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl Response {
    fn success(id: u64, result: serde_json::Value) -> Self {
        Response { id, ok: true, result: Some(result), error: None }
    }
    fn failure(id: u64, error: String) -> Self {
        Response { id, ok: false, result: None, error: Some(error) }
    }
}

fn dispatch(req: &Request) -> Response {
    let result: Result<serde_json::Value> = (|| {
        match req.method.as_str() {
            "ping" => Ok(serde_json::json!({ "version": env!("CARGO_PKG_VERSION") })),
            "machines" => Ok(serde_json::to_value(list_machines()?)?),
            "recent" => Ok(serde_json::to_value(State::load()?.recent)?),
            "add_machine" => {
                let name = req.params["name"].as_str().unwrap_or("").to_string();
                let address = req.params["address"].as_str().unwrap_or("");
                let mut target = SshTarget::parse(address)?;
                if !name.is_empty() {
                    target.name = Some(name.clone());
                }
                let mut registry = MachineRegistry::load()?;
                registry.add(
                    if name.is_empty() { target.authority() } else { name },
                    target,
                );
                registry.save()?;
                Ok(serde_json::json!({ "added": true }))
            }
            "connect" => {
                let target = resolve_target(param_str(&req.params, "machine")?)?;
                let mut progress = Vec::new();
                connect(&target, &mut progress)?;
                Ok(serde_json::json!({ "connected": true, "progress": progress }))
            }
            "discover" => {
                let target = resolve_target(param_str(&req.params, "machine")?)?;
                let d = discover(&target, None)?;
                Ok(serde_json::to_value(d)?)
            }
            "open" => {
                let target = resolve_target(param_str(&req.params, "machine")?)?;
                let result = open(&OpenRequest {
                    target,
                    environment: param_opt_str(&req.params, "environment"),
                    project: param_opt_str(&req.params, "project"),
                    no_launch: req.params["no_launch"].as_bool().unwrap_or(false),
                })?;
                Ok(serde_json::to_value(result)?)
            }
            "reopen" => {
                let result = reopen(
                    param_opt_str(&req.params, "environment").as_deref(),
                    req.params["no_launch"].as_bool().unwrap_or(false),
                )?;
                Ok(serde_json::to_value(result)?)
            }
            "rebuild" => {
                let target = resolve_target(param_str(&req.params, "machine")?)?;
                let progress = rebuild(&target, param_str(&req.params, "environment")?)?;
                Ok(serde_json::json!({ "progress": progress }))
            }
            "logs" => {
                let target = resolve_target(param_str(&req.params, "machine")?)?;
                let text = logs(&target, param_str(&req.params, "environment")?, 100)?;
                Ok(serde_json::json!({ "logs": text }))
            }
            "diagnose" => {
                let target = resolve_target(param_str(&req.params, "machine")?)?;
                let shell = RemoteShell::new(target);
                Ok(serde_json::to_value(crate::diagnostics::collect(&shell))?)
            }
            other => Err(anyhow!("unknown method '{other}'")),
        }
    })();
    match result {
        Ok(value) => Response::success(req.id, value),
        Err(e) => Response::failure(req.id, format!("{e:#}")),
    }
}

fn param_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    params[key]
        .as_str()
        .ok_or_else(|| anyhow!("missing parameter '{key}'"))
}

fn param_opt_str(params: &serde_json::Value, key: &str) -> Option<String> {
    params[key].as_str().map(|s| s.to_string())
}

/// Serve JSON-lines requests on stdin until EOF.
pub fn serve() -> Result<()> {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => dispatch(&req),
            Err(e) => Response {
                id: 0,
                ok: false,
                result: None,
                error: Some(format!("invalid request: {e}")),
            },
        };
        let mut out = stdout.lock();
        serde_json::to_writer(&mut out, &response)?;
        writeln!(out)?;
        out.flush()?;
    }
    Ok(())
}

/// Keep `workspace` referenced: workspace mapping helpers are exercised
/// through discovery, and re-exported here for integration tests.
#[allow(unused_imports)]
use workspace as _workspace;

//! Diagnostics: safe, opt-in visibility into the infrastructure.
//!
//! Primary error messages never expose raw SSH/Docker output; users can
//! always run `zrd diagnose <machine>` to inspect what is actually
//! happening underneath.

use crate::ssh::RemoteShell;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Diagnostics {
    pub machine: String,
    pub ssh: SshDiagnostics,
    pub remote: Option<RemoteDiagnostics>,
    pub local: LocalDiagnostics,
}

#[derive(Debug, Serialize)]
pub struct SshDiagnostics {
    /// The exact probe command, for advanced users.
    pub probe_command: String,
    pub reachable: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct RemoteDiagnostics {
    pub uname: String,
    pub docker: Option<String>,
    pub devcontainer_cli: bool,
    pub zed_server_installed: bool,
}

#[derive(Debug, Serialize)]
pub struct LocalDiagnostics {
    pub zed_cli_on_path: bool,
    pub zed_cli_override: Option<String>,
}

pub fn collect(shell: &RemoteShell) -> Diagnostics {
    let probe = "echo ok";
    let ssh_result = shell.run(probe);
    let reachable = ssh_result.is_ok();

    let remote = if reachable {
        let uname = shell
            .probe("uname -srm 2>/dev/null")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".into());
        let docker = shell
            .probe("docker --version 2>/dev/null")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let devcontainer_cli = crate::devcontainer::devcontainer_cli_available(shell);
        let zed_server_installed = shell
            .probe("ls ~/.zed_server/zed-remote-server-* >/dev/null 2>&1 && echo yes")
            .map(|s| s.trim() == "yes")
            .unwrap_or(false);
        Some(RemoteDiagnostics {
            uname,
            docker,
            devcontainer_cli,
            zed_server_installed,
        })
    } else {
        None
    };

    let zed_cli_override = std::env::var(crate::zed_launcher::ZED_BIN_ENV).ok();
    let zed_cli_on_path = std::process::Command::new(
        zed_cli_override.clone().unwrap_or_else(|| "zed".into()),
    )
    .arg("--version")
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false);

    Diagnostics {
        machine: shell.target().display_name(),
        ssh: SshDiagnostics {
            probe_command: shell.describe_command(probe),
            reachable,
            detail: match ssh_result {
                Ok(_) => "connected".into(),
                Err(e) => format!("{e:#}"),
            },
        },
        remote,
        local: LocalDiagnostics {
            zed_cli_on_path,
            zed_cli_override,
        },
    }
}

pub fn render_human(diag: &Diagnostics) -> String {
    let mut out = format!("Diagnostics for {}\n\n", diag.machine);
    out.push_str(&format!(
        "SSH:        {}\n  command:  {}\n",
        if diag.ssh.reachable { "ok" } else { "FAILED" },
        diag.ssh.probe_command
    ));
    if !diag.ssh.reachable {
        out.push_str(&format!("  detail:   {}\n", diag.ssh.detail));
    }
    if let Some(remote) = &diag.remote {
        out.push_str(&format!("Remote:     {}\n", remote.uname));
        out.push_str(&format!(
            "Docker:     {}\n",
            remote.docker.as_deref().unwrap_or("not available")
        ));
        out.push_str(&format!(
            "devcontainer CLI: {}\n",
            if remote.devcontainer_cli { "yes" } else { "no" }
        ));
        out.push_str(&format!(
            "Zed remote server: {}\n",
            if remote.zed_server_installed {
                "installed"
            } else {
                "not yet (Zed installs it on first connect)"
            }
        ));
    }
    out.push_str(&format!(
        "Zed CLI (local): {}\n",
        if diag.local.zed_cli_on_path {
            "available"
        } else {
            "NOT FOUND — install the Zed CLI to enable automatic open"
        }
    ));
    out
}

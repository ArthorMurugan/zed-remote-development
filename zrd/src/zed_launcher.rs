//! The Zed hand-off layer.
//!
//! This module is the ONLY place that knows how zrd asks Zed to open a
//! remote project. Zed's documented mechanism is the CLI URL form
//! `zed ssh://[user@]host[:port]/<path>` (and the `zed://ssh/...` hotlink);
//! if Zed later exposes a proper extension API for opening remote projects,
//! only this implementation is replaced.

use crate::ssh::SshTarget;
use anyhow::{Context, Result};
use std::process::Command;

/// A description of a connection to hand to Zed.
#[derive(Debug, Clone)]
pub struct RemoteConnection {
    pub target: SshTarget,
}

/// The hand-off abstraction. Keep Zed CLI assumptions inside this trait's
/// implementations — never spread them across the codebase.
pub trait ZedLauncher {
    fn open_remote_project(
        &self,
        connection: &RemoteConnection,
        workspace: &str,
    ) -> Result<()>;
}

/// Build the documented `ssh://` URL for a remote workspace.
///
/// Format: `ssh://[user@]host[:port]/absolute/path`
pub fn zed_ssh_url(target: &SshTarget, workspace: &str) -> String {
    let path = if workspace.starts_with('/') {
        workspace.to_string()
    } else {
        format!("/{workspace}")
    };
    match (target.user.as_deref(), target.port) {
        (Some(user), Some(port)) => format!("ssh://{user}@{}:{port}{path}", target.host),
        (Some(user), None) => format!("ssh://{user}@{}{path}", target.host),
        (None, Some(port)) => format!("ssh://{}:{port}{path}", target.host),
        (None, None) => format!("ssh://{}{path}", target.host),
    }
}

/// The `zed://` hotlink equivalent, for UIs that open URLs.
pub fn zed_hotlink(target: &SshTarget, workspace: &str) -> String {
    zed_ssh_url(target, workspace).replacen("ssh://", "zed://ssh/", 1)
}

/// Environment variable overriding the zed binary, for tests and for users
/// whose CLI is not on PATH.
pub const ZED_BIN_ENV: &str = "ZRD_ZED_BIN";

/// Launch Zed through its CLI: `zed ssh://user@host/path`.
pub struct CliZedLauncher {
    zed_bin: String,
}

impl Default for CliZedLauncher {
    fn default() -> Self {
        Self {
            zed_bin: std::env::var(ZED_BIN_ENV).unwrap_or_else(|_| "zed".to_string()),
        }
    }
}

impl ZedLauncher for CliZedLauncher {
    fn open_remote_project(
        &self,
        connection: &RemoteConnection,
        workspace: &str,
    ) -> Result<()> {
        let url = zed_ssh_url(&connection.target, workspace);
        Command::new(&self.zed_bin)
            .arg(&url)
            .spawn()
            .with_context(|| {
                format!(
                    "failed to launch Zed ('{}'). Is the Zed CLI installed? \
                     On macOS run `cli: install cli binary` from Zed's command palette.",
                    self.zed_bin
                )
            })?;
        Ok(())
    }
}

/// Open the `zed://` hotlink via the platform URL opener. Fallback for
/// environments where spawning the CLI directly is awkward.
pub struct UrlZedLauncher;

impl ZedLauncher for UrlZedLauncher {
    fn open_remote_project(
        &self,
        connection: &RemoteConnection,
        workspace: &str,
    ) -> Result<()> {
        let url = zed_hotlink(&connection.target, workspace);
        open_url(&url)
    }
}

#[cfg(target_os = "macos")]
fn open_url(url: &str) -> Result<()> {
    Command::new("open").arg(url).spawn()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_url(url: &str) -> Result<()> {
    Command::new("xdg-open").arg(url).spawn()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_url(url: &str) -> Result<()> {
    Command::new("cmd").args(["/C", "start", url]).spawn()?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn open_url(_url: &str) -> Result<()> {
    anyhow::bail!("opening zed:// URLs is not supported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(user: Option<&str>, host: &str, port: Option<u16>) -> SshTarget {
        SshTarget {
            host: host.into(),
            user: user.map(|u| u.into()),
            port,
            name: None,
            extra_args: vec![],
        }
    }

    #[test]
    fn generates_ssh_url_with_user_and_port() {
        let t = target(Some("dev"), "server.example.com", Some(2222));
        assert_eq!(
            zed_ssh_url(&t, "/workspace/api"),
            "ssh://dev@server.example.com:2222/workspace/api"
        );
    }

    #[test]
    fn generates_ssh_url_without_user() {
        let t = target(None, "gpu.internal", None);
        assert_eq!(zed_ssh_url(&t, "/home/researcher/proj"), "ssh://gpu.internal/home/researcher/proj");
    }

    #[test]
    fn normalizes_relative_workspace_to_absolute() {
        let t = target(Some("dev"), "host", None);
        assert_eq!(zed_ssh_url(&t, "workspace/api"), "ssh://dev@host/workspace/api");
    }

    #[test]
    fn generates_zed_hotlink() {
        let t = target(Some("dev"), "server.example.com", None);
        assert_eq!(
            zed_hotlink(&t, "/workspace/api"),
            "zed://ssh/dev@server.example.com/workspace/api"
        );
    }
}

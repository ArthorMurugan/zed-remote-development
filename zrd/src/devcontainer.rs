//! Dev Container discovery and lifecycle.
//!
//! zrd reads `.devcontainer/devcontainer.json` / `.devcontainer.json` on the
//! remote machine and orchestrates the `devcontainer` CLI when available,
//! falling back to plain `docker` for containers created from a config.

use crate::docker;
use crate::models::{Environment, EnvironmentKind, EnvironmentState};
use crate::ssh::RemoteShell;
use anyhow::{Context, Result};
use serde::Deserialize;

/// The subset of devcontainer.json zrd resolves.
///
/// devcontainer.json officially allows `//` and `/* */` comments, so parsing
/// goes through [`strip_json_comments`] first.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DevContainerConfig {
    pub name: Option<String>,
    pub image: Option<String>,
    pub docker_file: Option<String>,
    pub workspace_folder: Option<String>,
    pub workspace_mount: Option<String>,
    pub build: Option<DevContainerBuild>,
    pub post_start_command: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DevContainerBuild {
    pub dockerfile: Option<String>,
    pub context: Option<String>,
}

impl DevContainerConfig {
    pub fn parse(text: &str) -> Result<Self> {
        let stripped = strip_json_comments(text);
        serde_json::from_str(&stripped).context("failed to parse devcontainer.json")
    }

    /// The workspace path inside the container.
    pub fn container_workspace(&self) -> Option<String> {
        self.workspace_folder.clone()
    }
}

/// Remove `//` and `/* */` comments without touching string literals.
pub fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            '/' if i + 1 < bytes.len() && bytes[i + 1] as char == '/' => {
                while i < bytes.len() && bytes[i] as char != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < bytes.len() && bytes[i + 1] as char == '*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] as char == '*' && bytes[i + 1] as char == '/')
                {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Look for a devcontainer config at the usual locations below `project_path`
/// on the remote machine. Returns the config and the config directory.
pub fn find_devcontainer(
    shell: &RemoteShell,
    project_path: &str,
) -> Result<Option<(DevContainerConfig, String)>> {
    let candidates = [
        format!("{project_path}/.devcontainer/devcontainer.json"),
        format!("{project_path}/.devcontainer.json"),
    ];
    for candidate in &candidates {
        let cmd = format!("cat {} 2>/dev/null", docker::shell_escape(candidate));
        if let Some(text) = shell.probe(&cmd) {
            if text.trim().is_empty() {
                continue;
            }
            let config = DevContainerConfig::parse(&text)
                .with_context(|| format!("invalid devcontainer config at {candidate}"))?;
            let dir = candidate
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_else(|| project_path.to_string());
            return Ok(Some((config, dir)));
        }
    }
    Ok(None)
}

/// Whether the Dev Containers CLI is available remotely.
pub fn devcontainer_cli_available(shell: &RemoteShell) -> bool {
    shell
        .probe("command -v devcontainer >/dev/null 2>&1 && echo ok")
        .map(|out| out.trim() == "ok")
        .unwrap_or(false)
}

/// Build/start the devcontainer for a project via the official CLI.
pub fn up(shell: &RemoteShell, project_path: &str) -> Result<()> {
    shell
        .run(&format!(
            "devcontainer up --workspace-folder {}",
            docker::shell_escape(project_path)
        ))
        .context("the development environment failed to build or start")?;
    Ok(())
}

/// Rebuild the devcontainer from scratch.
pub fn rebuild(shell: &RemoteShell, project_path: &str) -> Result<()> {
    shell
        .run(&format!(
            "devcontainer up --remove-existing-container --build-no-cache --workspace-folder {}",
            docker::shell_escape(project_path)
        ))
        .context("the development environment failed to rebuild")?;
    Ok(())
}

/// Describe a devcontainer config as an environment.
pub fn to_environment(config: &DevContainerConfig, project_path: &str) -> Environment {
    let name = config.name.clone().unwrap_or_else(|| {
        project_path
            .rsplit('/')
            .next()
            .unwrap_or("devcontainer")
            .to_string()
    });
    let description = config
        .image
        .clone()
        .or_else(|| config.docker_file.as_ref().map(|d| format!("build: {d}")))
        .or_else(|| {
            config
                .build
                .as_ref()
                .and_then(|b| b.dockerfile.clone())
                .map(|d| format!("build: {d}"))
        });
    Environment {
        name,
        kind: EnvironmentKind::DevContainer,
        state: EnvironmentState::Unknown, // refined by docker discovery
        description,
        project_path: Some(project_path.to_string()),
        workspace: Some(project_path.to_string()),
        handle: Some(project_path.to_string()),
    }
}

#[allow(unused)]
fn _state_marker(_: EnvironmentState) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devcontainer_json_with_comments() {
        let text = r#"{
            // API service container
            "name": "api",
            "image": "mcr.microsoft.com/devcontainers/javascript-node:20",
            /* workspace inside the container */
            "workspaceFolder": "/workspace/api",
            "postStartCommand": "pnpm install"
        }"#;
        let cfg = DevContainerConfig::parse(text).unwrap();
        assert_eq!(cfg.name.as_deref(), Some("api"));
        assert_eq!(cfg.workspace_folder.as_deref(), Some("/workspace/api"));
    }

    #[test]
    fn comment_stripper_preserves_urls_in_strings() {
        let text = r#"{"image": "https://example.com/image"}"#;
        let stripped = strip_json_comments(text);
        assert!(stripped.contains("https://example.com/image"));
    }

    #[test]
    fn parses_build_configuration() {
        let text = r#"{
            "name": "rust-service",
            "build": { "dockerfile": "Dockerfile", "context": ".." }
        }"#;
        let cfg = DevContainerConfig::parse(text).unwrap();
        assert!(cfg.image.is_none());
        assert_eq!(
            cfg.build.as_ref().and_then(|b| b.dockerfile.clone()).as_deref(),
            Some("Dockerfile")
        );
    }

    #[test]
    fn environment_uses_config_name_not_container_id() {
        let cfg = DevContainerConfig::parse(
            r#"{"name": "api", "image": "node:20", "workspaceFolder": "/workspace/api"}"#,
        )
        .unwrap();
        let env = to_environment(&cfg, "/home/dev/api");
        assert_eq!(env.name, "api");
        assert_eq!(env.kind, EnvironmentKind::DevContainer);
        assert_eq!(env.workspace.as_deref(), Some("/home/dev/api"));
    }
}

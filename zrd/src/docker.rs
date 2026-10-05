//! Docker discovery and lifecycle on the remote machine.
//!
//! zrd orchestrates the remote `docker` CLI over SSH; it never speaks the
//! Docker socket protocol itself.

use crate::models::{Environment, EnvironmentKind, EnvironmentState};
use crate::ssh::RemoteShell;
use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ContainerMount {
    #[serde(rename = "Source")]
    pub source: String,
    #[serde(rename = "Destination")]
    pub destination: String,
}

/// The subset of `docker inspect` output zrd cares about.
#[derive(Debug, Clone, Deserialize)]
pub struct ContainerInspect {
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(default, rename = "Config")]
    pub config: InspectConfig,
    #[serde(default, rename = "State")]
    pub state: InspectState,
    #[serde(default, rename = "Mounts")]
    pub mounts: Vec<ContainerMount>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InspectConfig {
    #[serde(rename = "Image", default)]
    pub image: String,
    #[serde(rename = "WorkingDir", default)]
    pub working_dir: String,
    #[serde(rename = "Labels", default)]
    pub labels: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InspectState {
    #[serde(rename = "Status", default)]
    pub status: String,
    #[serde(rename = "Running", default)]
    pub running: bool,
}

/// A container discovered via `docker ps -a --format '{{json .}}'`.
#[derive(Debug, Clone, Deserialize)]
pub struct DockerPsEntry {
    #[serde(rename = "ID", default)]
    pub id: String,
    #[serde(rename = "Names", default)]
    pub names: String,
    #[serde(rename = "Image", default)]
    pub image: String,
    #[serde(rename = "State", default)]
    pub state: String,
    #[serde(rename = "Status", default)]
    pub status: String,
    #[serde(rename = "Labels", default)]
    pub labels: String,
    #[serde(rename = "Mounts", default)]
    pub mounts: String,
}

pub fn docker_available(shell: &RemoteShell) -> bool {
    shell
        .probe("command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1 && echo ok")
        .map(|out| out.trim() == "ok")
        .unwrap_or(false)
}

/// List all containers (running and stopped).
pub fn list_containers(shell: &RemoteShell) -> Result<Vec<DockerPsEntry>> {
    let out = shell
        .run("docker ps -a --format '{{json .}}'")
        .context("failed to list Docker containers")?;
    Ok(parse_docker_ps(&out))
}

/// Parse `docker ps --format '{{json .}}'` output (one JSON object per line).
pub fn parse_docker_ps(output: &str) -> Vec<DockerPsEntry> {
    output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<DockerPsEntry>(l).ok())
        .collect()
}

/// Inspect a single container for mounts, labels, workdir and state.
pub fn inspect_container(shell: &RemoteShell, name: &str) -> Result<ContainerInspect> {
    let out = shell
        .run(&format!("docker inspect {}", shell_escape(name)))
        .with_context(|| format!("failed to inspect environment '{name}'"))?;
    let mut items: Vec<ContainerInspect> =
        serde_json::from_str(&out).context("unexpected docker inspect output")?;
    items
        .pop()
        .ok_or_else(|| anyhow::anyhow!("environment '{name}' not found"))
}

/// Map a `docker ps` state string onto the explicit environment state model.
pub fn state_from_ps(state: &str) -> EnvironmentState {
    match state.to_ascii_lowercase().as_str() {
        "running" => EnvironmentState::Running,
        "exited" | "created" | "paused" | "dead" => EnvironmentState::Stopped,
        "restarting" => EnvironmentState::Starting,
        _ => EnvironmentState::Unknown,
    }
}

/// Turn a discovered container into an environment with a human name.
pub fn container_to_environment(entry: &DockerPsEntry) -> Environment {
    let name = entry.names.trim_start_matches('/').to_string();
    Environment {
        name: name.clone(),
        kind: EnvironmentKind::Docker,
        state: state_from_ps(&entry.state),
        description: if entry.image.is_empty() {
            None
        } else {
            Some(entry.image.clone())
        },
        project_path: None,
        workspace: None,
        handle: Some(name),
    }
}

pub fn start_container(shell: &RemoteShell, name: &str) -> Result<()> {
    shell
        .run(&format!("docker start {}", shell_escape(name)))
        .with_context(|| format!("the development environment '{name}' failed to start"))?;
    Ok(())
}

pub fn stop_container(shell: &RemoteShell, name: &str) -> Result<()> {
    shell
        .run(&format!("docker stop {}", shell_escape(name)))
        .with_context(|| format!("the development environment '{name}' failed to stop"))?;
    Ok(())
}

pub fn restart_container(shell: &RemoteShell, name: &str) -> Result<()> {
    shell
        .run(&format!("docker restart {}", shell_escape(name)))
        .with_context(|| format!("the development environment '{name}' failed to restart"))?;
    Ok(())
}

pub fn container_logs(shell: &RemoteShell, name: &str, tail: u32) -> Result<String> {
    shell
        .run(&format!(
            "docker logs --tail {} {} 2>&1",
            tail,
            shell_escape(name)
        ))
        .with_context(|| format!("could not read logs for '{name}'"))
}

/// Shell-escape a single argument for the remote shell.
pub fn shell_escape(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_ps_json_lines() {
        let out = concat!(
            r#"{"ID":"4f7a8c","Names":"api","Image":"node:20","State":"running","Status":"Up 2 hours","Labels":"","Mounts":"/home/dev/api"}"#,
            "\n",
            r#"{"ID":"9aa1","Names":"postgres","Image":"postgres:16","State":"exited","Status":"Exited (0) 3 days ago","Labels":"","Mounts":""}"#,
            "\n"
        );
        let entries = parse_docker_ps(out);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].names, "api");
        assert_eq!(entries[1].state, "exited");
    }

    #[test]
    fn maps_ps_state_to_environment_state() {
        assert_eq!(state_from_ps("running"), EnvironmentState::Running);
        assert_eq!(state_from_ps("exited"), EnvironmentState::Stopped);
        assert_eq!(state_from_ps("restarting"), EnvironmentState::Starting);
        assert_eq!(state_from_ps("weird"), EnvironmentState::Unknown);
    }

    #[test]
    fn container_becomes_human_named_environment() {
        let entry = DockerPsEntry {
            id: "4f7a8c".into(),
            names: "api".into(),
            image: "node:20".into(),
            state: "running".into(),
            status: "Up".into(),
            labels: String::new(),
            mounts: String::new(),
        };
        let env = container_to_environment(&entry);
        // The user sees "api", never "container 4f7a8c".
        assert_eq!(env.name, "api");
        assert_eq!(env.description.as_deref(), Some("node:20"));
        assert_eq!(env.state, EnvironmentState::Running);
    }

    #[test]
    fn shell_escape_quotes_safely() {
        assert_eq!(shell_escape("api"), "'api'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }
}

//! Docker Compose discovery and lifecycle on the remote machine.

use crate::docker;
use crate::models::{Environment, EnvironmentKind, EnvironmentState};
use crate::ssh::RemoteShell;
use anyhow::{Context, Result};
use serde::Deserialize;

/// Canonical compose file names, in search order.
pub const COMPOSE_FILE_NAMES: [&str; 4] = [
    "compose.yml",
    "compose.yaml",
    "docker-compose.yml",
    "docker-compose.yaml",
];

/// The subset of a compose file zrd resolves.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ComposeFile {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub services: std::collections::HashMap<String, ComposeService>,
}

/// A single compose service.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ComposeService {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub build: Option<serde_yaml::Value>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub volumes: Vec<serde_yaml::Value>,
}

/// A compose project discovered on the remote machine.
#[derive(Debug, Clone)]
pub struct ComposeProject {
    /// Directory containing the compose file.
    pub directory: String,
    /// The compose file that was found.
    pub file_name: String,
    /// Project name: explicit `name:` or the directory name.
    pub name: String,
    pub services: Vec<String>,
}

impl ComposeFile {
    pub fn parse(text: &str) -> Result<Self> {
        serde_yaml::from_str(text).context("failed to parse compose file")
    }
}

/// Look for a compose file directly inside `project_path`.
pub fn find_compose_file(shell: &RemoteShell, project_path: &str) -> Result<Option<String>> {
    for name in COMPOSE_FILE_NAMES {
        let path = format!("{project_path}/{name}");
        let cmd = format!("test -f {} && echo yes", docker::shell_escape(&path));
        if shell.probe(&cmd).map(|o| o.trim() == "yes").unwrap_or(false) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Read and resolve a compose project from a compose file path.
pub fn load_compose_project(shell: &RemoteShell, compose_path: &str) -> Result<ComposeProject> {
    let text = shell
        .run(&format!("cat {}", docker::shell_escape(compose_path)))
        .with_context(|| format!("could not read {compose_path}"))?;
    let parsed = ComposeFile::parse(&text)?;
    let (directory, file_name) = compose_path
        .rsplit_once('/')
        .map(|(d, f)| (d.to_string(), f.to_string()))
        .unwrap_or_else(|| (".".into(), compose_path.to_string()));
    let name = parsed.name.clone().unwrap_or_else(|| {
        directory
            .rsplit('/')
            .next()
            .unwrap_or("compose-project")
            .to_string()
    });
    Ok(ComposeProject {
        directory,
        file_name,
        name,
        services: parsed.services.keys().cloned().collect(),
    })
}

/// Query service state via `docker compose ps` when available.
pub fn compose_services_running(
    shell: &RemoteShell,
    project_dir: &str,
    file_name: &str,
) -> Vec<String> {
    let cmd = format!(
        "cd {} && docker compose -f {} ps --services --filter status=running 2>/dev/null",
        docker::shell_escape(project_dir),
        docker::shell_escape(file_name)
    );
    shell
        .probe(&cmd)
        .map(|out| out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// Bring up the compose project (detached).
pub fn up(shell: &RemoteShell, project: &ComposeProject) -> Result<()> {
    shell
        .run(&format!(
            "cd {} && docker compose -f {} up -d",
            docker::shell_escape(&project.directory),
            docker::shell_escape(&project.file_name)
        ))
        .with_context(|| format!("the development environment '{}' failed to start", project.name))
        .map(|_| ())
}

/// Rebuild and restart the compose project.
pub fn rebuild(shell: &RemoteShell, project: &ComposeProject) -> Result<()> {
    shell
        .run(&format!(
            "cd {} && docker compose -f {} up -d --build",
            docker::shell_escape(&project.directory),
            docker::shell_escape(&project.file_name)
        ))
        .with_context(|| {
            format!("the development environment '{}' failed to rebuild", project.name)
        })
        .map(|_| ())
}

/// Describe a compose project as an environment.
pub fn to_environment(project: &ComposeProject, running_services: &[String]) -> Environment {
    let state = if running_services.is_empty() {
        EnvironmentState::Stopped
    } else {
        EnvironmentState::Running
    };
    let description = if project.services.is_empty() {
        None
    } else {
        Some(format!("{} services", project.services.len()))
    };
    Environment {
        name: project.name.clone(),
        kind: EnvironmentKind::Compose,
        state,
        description,
        project_path: Some(project.directory.clone()),
        workspace: Some(project.directory.clone()),
        handle: Some(format!("{}:{}", project.directory, project.file_name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compose_services() {
        let text = r#"
name: my-project
services:
  api:
    image: node:20
    working_dir: /workspace/api
    volumes:
      - ./api:/workspace/api
  db:
    image: postgres:16
"#;
        let file = ComposeFile::parse(text).unwrap();
        assert_eq!(file.name.as_deref(), Some("my-project"));
        assert!(file.services.contains_key("api"));
        assert!(file.services.contains_key("db"));
        assert_eq!(
            file.services["api"].working_dir.as_deref(),
            Some("/workspace/api")
        );
    }

    #[test]
    fn compose_project_name_falls_back_to_directory() {
        let file = ComposeFile::parse("services:\n  web:\n    image: nginx\n").unwrap();
        assert!(file.name.is_none());
        assert!(file.services.contains_key("web"));
    }

    #[test]
    fn environment_state_reflects_running_services() {
        let project = ComposeProject {
            directory: "/home/dev/my-project".into(),
            file_name: "compose.yml".into(),
            name: "my-project".into(),
            services: vec!["api".into(), "db".into()],
        };
        let stopped = to_environment(&project, &[]);
        assert_eq!(stopped.state, EnvironmentState::Stopped);
        let running = to_environment(&project, &["api".to_string()]);
        assert_eq!(running.state, EnvironmentState::Running);
        assert_eq!(running.description.as_deref(), Some("2 services"));
    }
}

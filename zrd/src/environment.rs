//! Environment lifecycle: state machine and start/stop/rebuild orchestration.
//!
//! The lifecycle never blocks a UI: every operation is a discrete step whose
//! result is reported through progress lines and structured errors.

use crate::compose;
use crate::devcontainer;
use crate::docker;
use crate::models::{Environment, EnvironmentKind, EnvironmentState};
use crate::ssh::RemoteShell;
use anyhow::{anyhow, Result};

/// Refresh the state of an environment.
pub fn refresh_state(shell: &RemoteShell, env: &mut Environment) -> Result<()> {
    match env.kind {
        EnvironmentKind::Docker => {
            let name = env
                .handle
                .clone()
                .ok_or_else(|| anyhow!("environment has no container handle"))?;
            let inspect = docker::inspect_container(shell, &name)?;
            env.state = if inspect.state.running {
                EnvironmentState::Running
            } else {
                EnvironmentState::Stopped
            };
        }
        EnvironmentKind::DevContainer => {
            // A devcontainer's state is tracked by the container it maps to,
            // when one exists. Otherwise it is treated as stopped (buildable).
            env.state = match &env.handle {
                Some(handle) if docker::inspect_container(shell, handle).is_ok() => {
                    let inspect = docker::inspect_container(shell, handle)?;
                    if inspect.state.running {
                        EnvironmentState::Running
                    } else {
                        EnvironmentState::Stopped
                    }
                }
                _ => EnvironmentState::Stopped,
            };
        }
        EnvironmentKind::Compose => {
            let handle = env
                .handle
                .clone()
                .ok_or_else(|| anyhow!("environment has no compose handle"))?;
            let (dir, file) = handle
                .split_once(':')
                .ok_or_else(|| anyhow!("invalid compose handle"))?;
            let running = compose::compose_services_running(shell, dir, file);
            env.state = if running.is_empty() {
                EnvironmentState::Stopped
            } else {
                EnvironmentState::Running
            };
        }
    }
    Ok(())
}

/// Ensure an environment is running, starting or building it if necessary.
/// Returns progress lines describing what happened.
pub fn ensure_running(
    shell: &RemoteShell,
    env: &mut Environment,
    progress: &mut Vec<String>,
) -> Result<()> {
    refresh_state(shell, env)?;
    match env.state {
        EnvironmentState::Running => {
            progress.push(format!("'{}' is already running", env.name));
            Ok(())
        }
        EnvironmentState::Stopped | EnvironmentState::Unknown | EnvironmentState::Failed => {
            progress.push(format!("Starting '{}'...", env.name));
            env.state = EnvironmentState::Starting;
            start(shell, env)?;
            env.state = EnvironmentState::Running;
            progress.push(format!("'{}' started", env.name));
            Ok(())
        }
        other => Err(anyhow!(
            "the development environment '{}' is {other} and cannot be opened right now",
            env.name
        )),
    }
}

/// Start an environment using the right backend for its kind.
pub fn start(shell: &RemoteShell, env: &Environment) -> Result<()> {
    match env.kind {
        EnvironmentKind::Docker => {
            let name = env.handle.as_deref().unwrap_or(&env.name);
            docker::start_container(shell, name)
        }
        EnvironmentKind::DevContainer => {
            let project = env
                .project_path
                .as_deref()
                .ok_or_else(|| anyhow!("devcontainer environment has no project path"))?;
            if devcontainer::devcontainer_cli_available(shell) {
                devcontainer::up(shell, project)
            } else if let Some(handle) = &env.handle {
                // Fall back to plain docker when the container already exists.
                docker::start_container(shell, handle)
            } else {
                Err(anyhow!(
                    "the Dev Containers CLI is not installed on this machine, \
                     and no existing container matches '{}'",
                    env.name
                ))
            }
        }
        EnvironmentKind::Compose => {
            let handle = env
                .handle
                .as_deref()
                .ok_or_else(|| anyhow!("compose environment has no handle"))?;
            let (dir, file) = handle
                .split_once(':')
                .ok_or_else(|| anyhow!("invalid compose handle"))?;
            let project = compose::ComposeProject {
                directory: dir.to_string(),
                file_name: file.to_string(),
                name: env.name.clone(),
                services: Vec::new(),
            };
            compose::up(shell, &project)
        }
    }
}

/// Rebuild an environment (devcontainer rebuild / compose --build /
/// docker restart as the no-op equivalent for plain containers).
pub fn rebuild(shell: &RemoteShell, env: &Environment, progress: &mut Vec<String>) -> Result<()> {
    progress.push(format!("Rebuilding '{}'...", env.name));
    match env.kind {
        EnvironmentKind::Docker => {
            // Plain containers have no build step; restart is the honest
            // equivalent.
            docker::restart_container(shell, env.handle.as_deref().unwrap_or(&env.name))
        }
        EnvironmentKind::DevContainer => {
            let project = env
                .project_path
                .as_deref()
                .ok_or_else(|| anyhow!("devcontainer environment has no project path"))?;
            devcontainer::rebuild(shell, project)
        }
        EnvironmentKind::Compose => {
            let handle = env
                .handle
                .as_deref()
                .ok_or_else(|| anyhow!("compose environment has no handle"))?;
            let (dir, file) = handle
                .split_once(':')
                .ok_or_else(|| anyhow!("invalid compose handle"))?;
            let project = compose::ComposeProject {
                directory: dir.to_string(),
                file_name: file.to_string(),
                name: env.name.clone(),
                services: Vec::new(),
            };
            compose::rebuild(shell, &project)
        }
    }
    .map(|_| progress.push(format!("'{}' rebuilt", env.name)))
}

/// Read recent logs for an environment.
pub fn logs(shell: &RemoteShell, env: &Environment, tail: u32) -> Result<String> {
    match env.kind {
        EnvironmentKind::Docker | EnvironmentKind::DevContainer => {
            docker::container_logs(shell, env.handle.as_deref().unwrap_or(&env.name), tail)
        }
        EnvironmentKind::Compose => {
            let handle = env
                .handle
                .as_deref()
                .ok_or_else(|| anyhow!("compose environment has no handle"))?;
            let (dir, file) = handle
                .split_once(':')
                .ok_or_else(|| anyhow!("invalid compose handle"))?;
            shell.run(&format!(
                "cd {} && docker compose -f {} logs --tail {} 2>&1",
                docker::shell_escape(dir),
                docker::shell_escape(file),
                tail
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_state_display() {
        assert_eq!(EnvironmentState::Running.to_string(), "running");
        assert_eq!(EnvironmentState::Rebuilding.to_string(), "rebuilding");
    }
}

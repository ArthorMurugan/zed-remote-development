//! Workspace resolution.
//!
//! Zed opens remote projects over SSH against the *host* filesystem, so for
//! container-based environments zrd maps the in-container workspace back to
//! its bind-mounted host path. See ASSUMPTIONS.md for why.

use crate::devcontainer::DevContainerConfig;
use crate::docker::ContainerInspect;

/// Destinations that conventionally hold the workspace inside a container,
/// most specific first.
const WORKSPACE_DESTINATIONS: [&str; 4] =
    ["/workspaces", "/workspace", "/workdir", "/app"];

/// Resolve the host-side workspace for a Docker environment.
///
/// Preference order:
///   1. A bind mount whose destination looks like a workspace root
///      (`/workspaces/...`, `/workspace`, ...).
///   2. The container's configured `WorkingDir`, if it is bind-mounted.
///   3. `None` — the caller falls back to the project path.
pub fn resolve_container_workspace(inspect: &ContainerInspect) -> Option<String> {
    // 1. Conventional workspace mounts first.
    for dest_prefix in WORKSPACE_DESTINATIONS {
        if let Some(m) = inspect
            .mounts
            .iter()
            .find(|m| m.destination == dest_prefix || m.destination.starts_with(&format!("{dest_prefix}/")))
        {
            return Some(m.source.clone());
        }
    }
    // 2. WorkingDir, if it is backed by a bind mount.
    let workdir = inspect.config.working_dir.trim();
    if !workdir.is_empty() {
        if let Some(m) = inspect
            .mounts
            .iter()
            .find(|m| m.destination == workdir || workdir.starts_with(&format!("{}/", m.destination)))
        {
            // If WorkingDir is a subdirectory of the mount, map the subpath
            // back onto the host source so Zed opens the real project root.
            if workdir == m.destination {
                return Some(m.source.clone());
            }
            let sub = workdir[m.destination.len()..].trim_start_matches('/');
            return Some(format!("{}/{sub}", m.source.trim_end_matches('/')));
        }
    }
    None
}

/// Resolve the host-side workspace for a devcontainer.
///
/// A devcontainer's `workspaceFolder` lives *inside* the container; the host
/// workspace is simply the project directory that contains `.devcontainer`.
pub fn resolve_devcontainer_workspace(
    project_path: &str,
    _config: &DevContainerConfig,
) -> String {
    project_path.trim_end_matches('/').to_string()
}

/// Resolve the workspace for a compose project: the directory containing
/// the compose file (services mount subtrees of it in the common case).
pub fn resolve_compose_workspace(project_dir: &str) -> String {
    project_dir.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::{ContainerMount, InspectConfig, InspectState};

    fn inspect(mounts: Vec<(&str, &str)>, workdir: &str) -> ContainerInspect {
        ContainerInspect {
            name: "/api".into(),
            config: InspectConfig {
                image: "node:20".into(),
                working_dir: workdir.into(),
                labels: Default::default(),
            },
            state: InspectState::default(),
            mounts: mounts
                .into_iter()
                .map(|(s, d)| ContainerMount {
                    source: s.into(),
                    destination: d.into(),
                })
                .collect(),
        }
    }

    #[test]
    fn prefers_workspace_mount() {
        let i = inspect(
            vec![
                ("/var/lib/data", "/data"),
                ("/home/dev/api", "/workspace/api"),
            ],
            "",
        );
        assert_eq!(
            resolve_container_workspace(&i).as_deref(),
            Some("/home/dev/api")
        );
    }

    #[test]
    fn falls_back_to_mounted_working_dir() {
        let i = inspect(vec![("/home/dev/api", "/app")], "/app");
        assert_eq!(resolve_container_workspace(&i).as_deref(), Some("/home/dev/api"));
    }

    #[test]
    fn maps_working_dir_subpath_through_mount() {
        let i = inspect(vec![("/home/dev/repo", "/src")], "/src/api");
        assert_eq!(
            resolve_container_workspace(&i).as_deref(),
            Some("/home/dev/repo/api")
        );
    }

    #[test]
    fn returns_none_without_relevant_mounts() {
        let i = inspect(vec![("/var/lib/data", "/data")], "/app");
        assert_eq!(resolve_container_workspace(&i), None);
    }

    #[test]
    fn devcontainer_workspace_is_the_project_directory() {
        let cfg = DevContainerConfig::parse(
            r#"{"name":"api","workspaceFolder":"/workspace/api"}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_devcontainer_workspace("/home/dev/api", &cfg),
            "/home/dev/api"
        );
    }
}

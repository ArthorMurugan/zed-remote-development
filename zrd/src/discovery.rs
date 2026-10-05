//! Environment and project discovery on a connected machine.
//!
//! Discovery is deliberately shallow and rooted at a small set of sensible
//! directories — zrd never recursively scans huge filesystems.

use crate::activity;
use crate::compose;
use crate::devcontainer;
use crate::docker;
use crate::models::{Activity, Discovery, Environment, Project};
use crate::ssh::RemoteShell;
use crate::workspace;
use anyhow::Result;

/// Marker files that identify a project root.
pub const PROJECT_MARKERS: [&str; 11] = [
    ".git",
    "package.json",
    "pnpm-workspace.yaml",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "Makefile",
    "justfile",
    "Taskfile.yml",
    ".devcontainer",
    "compose.yml",
];

/// Default roots scanned for projects, in order. Roots that don't exist on
/// the remote are skipped.
pub const DEFAULT_SEARCH_ROOTS: [&str; 6] = [
    "~/code",
    "~/projects",
    "~/src",
    "~/work",
    "~",
    "/workspaces",
];

/// Default maximum directory depth below each search root.
pub const DEFAULT_MAX_DEPTH: u32 = 3;

/// Discover projects below the search roots without scanning deeply.
pub fn discover_projects(
    shell: &RemoteShell,
    roots: &[String],
    max_depth: u32,
) -> Result<Vec<Project>> {
    let roots_expr = roots
        .iter()
        .map(|r| docker::shell_escape(r))
        .collect::<Vec<_>>()
        .join(" ");
    // Build a find expression matching any marker name.
    let names = PROJECT_MARKERS
        .iter()
        .map(|m| format!("-name {}", docker::shell_escape(m)))
        .collect::<Vec<_>>()
        .join(" -o ");
    let cmd = format!(
        "for r in {roots_expr}; do d=$(eval echo $r); [ -d \"$d\" ] && \
         find \"$d\" -maxdepth {max_depth} \\( {names} \\) -print 2>/dev/null; done"
    );
    let out = shell.run(&cmd)?;
    Ok(projects_from_find_output(&out))
}

/// Group marker hits from `find` output into project roots.
pub fn projects_from_find_output(output: &str) -> Vec<Project> {
    let mut map: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for line in output.lines().map(str::trim).filter(|l| !l.is_empty()) {
        // A marker hit's project root is its parent directory; for
        // `.devcontainer/devcontainer.json` the root is two levels up.
        let root = if line.ends_with("/.devcontainer") {
            line.trim_end_matches("/.devcontainer").to_string()
        } else if line.contains("/.devcontainer/") {
            line.split("/.devcontainer/").next().unwrap_or(line).to_string()
        } else {
            line.rsplit_once('/')
                .map(|(dir, _)| dir.to_string())
                .unwrap_or_else(|| line.to_string())
        };
        if root.is_empty() {
            continue;
        }
        let marker = line
            .rsplit('/')
            .next()
            .unwrap_or(line)
            .to_string();
        map.entry(root).or_default().push(marker);
    }
    // If a directory is both a parent and a child project (e.g. a monorepo
    // root with package.json and nested packages), keep both — the picker
    // presents them by path, which is unambiguous.
    map.into_iter()
        .map(|(path, mut markers)| {
            markers.sort();
            markers.dedup();
            let name = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .to_string();
            Project { path, name, markers }
        })
        .collect()
}

/// Discover all development environments: Docker containers, devcontainers
/// in discovered projects, and compose projects.
pub fn discover_environments(
    shell: &RemoteShell,
    projects: &[Project],
    progress: &mut Vec<String>,
) -> Result<Vec<Environment>> {
    let mut environments = Vec::new();

    if docker::docker_available(shell) {
        progress.push("Docker detected".into());
        for entry in docker::list_containers(shell).unwrap_or_default() {
            let mut env = docker::container_to_environment(&entry);
            if let Ok(inspect) = docker::inspect_container(shell, &entry.names) {
                env.workspace = workspace::resolve_container_workspace(&inspect);
                env.project_path = env.workspace.clone();
            }
            environments.push(env);
        }
    } else {
        progress.push("Docker not available on this machine".into());
    }

    for project in projects {
        if let Ok(Some((config, _))) = devcontainer::find_devcontainer(shell, &project.path) {
            let mut env = devcontainer::to_environment(&config, &project.path);
            env.workspace = Some(workspace::resolve_devcontainer_workspace(
                &project.path,
                &config,
            ));
            // If a matching container is already known, reuse its state.
            if let Some(existing) = environments
                .iter()
                .find(|e| e.kind == crate::models::EnvironmentKind::Docker && e.name == env.name)
            {
                env.state = existing.state;
            }
            environments.push(env);
        }

        if let Ok(Some(compose_path)) = compose::find_compose_file(shell, &project.path) {
            if let Ok(project_meta) = compose::load_compose_project(shell, &compose_path) {
                let running = compose::compose_services_running(
                    shell,
                    &project_meta.directory,
                    &project_meta.file_name,
                );
                environments.push(compose::to_environment(&project_meta, &running));
            }
        }
    }

    progress.push(format!("{} environments discovered", environments.len()));
    Ok(environments)
}

/// Infer development activities for the discovered projects.
pub fn discover_activities(shell: &RemoteShell, projects: &[Project]) -> Vec<Activity> {
    projects
        .iter()
        .flat_map(|p| {
            activity::infer_activities(p, &|path: &str| {
                shell.probe(&format!("cat {}", docker::shell_escape(path)))
            })
        })
        .collect()
}

/// Full discovery pass for a connected machine.
pub fn discover(shell: &RemoteShell, roots: Option<Vec<String>>) -> Result<Discovery> {
    let mut progress = Vec::new();
    progress.push(format!("Connected to {}", shell.target().display_name()));

    let roots = roots.unwrap_or_else(|| DEFAULT_SEARCH_ROOTS.iter().map(|s| s.to_string()).collect());
    let projects = discover_projects(shell, &roots, DEFAULT_MAX_DEPTH)?;
    progress.push(format!("{} projects discovered", projects.len()));

    let environments = discover_environments(shell, &projects, &mut progress)?;
    let activities = discover_activities(shell, &projects);

    Ok(Discovery {
        machine: shell.target().display_name(),
        environments,
        projects,
        activities,
        progress,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_find_output_into_projects() {
        let out = concat!(
            "/home/dev/api/.git\n",
            "/home/dev/api/package.json\n",
            "/home/dev/api/.devcontainer/devcontainer.json\n",
            "/home/dev/frontend/package.json\n",
            "/home/dev/infra/compose.yml\n"
        );
        let projects = projects_from_find_output(out);
        assert_eq!(projects.len(), 3);
        let api = projects.iter().find(|p| p.name == "api").unwrap();
        assert_eq!(api.path, "/home/dev/api");
        assert!(api.markers.contains(&".git".to_string()));
        assert!(api.markers.contains(&"package.json".to_string()));
        assert!(api.markers.contains(&"devcontainer.json".to_string()));
    }

    #[test]
    fn ignores_empty_lines_and_root_hits() {
        let projects = projects_from_find_output("\n\n");
        assert!(projects.is_empty());
    }
}

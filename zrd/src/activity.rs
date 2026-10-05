//! Development Activity inference.
//!
//! An activity = environment + workspace + startup command (+ optional
//! ports/env). Activities are inferred from project metadata so users don't
//! have to define them by hand for normal projects.

use crate::models::{Activity, Project};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PackageJson {
    name: Option<String>,
    scripts: Option<std::collections::HashMap<String, String>>,
}

/// Well-known npm script names that start a development session, in
/// priority order.
const DEV_SCRIPT_NAMES: [&str; 4] = ["dev", "start", "serve", "develop"];

/// Infer activities for a discovered project from its metadata files.
///
/// `read_file` fetches a file's contents from the remote machine (already
/// SSH-mediated by the caller), which keeps this module pure and testable.
pub fn infer_activities(
    project: &Project,
    read_file: &dyn Fn(&str) -> Option<String>,
) -> Vec<Activity> {
    let mut activities = Vec::new();

    if project.markers.iter().any(|m| m == "package.json") {
        if let Some(a) = from_package_json(project, read_file) {
            activities.push(a);
        }
    }
    if project.markers.iter().any(|m| m == "Cargo.toml") {
        activities.push(from_cargo(project, read_file));
    }
    if project.markers.iter().any(|m| m == "go.mod") {
        activities.push(from_go(project));
    }
    if project.markers.iter().any(|m| m == "pyproject.toml") {
        activities.push(Activity {
            name: format!("{} Development", title_case(&project.name)),
            environment: None,
            workspace: project.path.clone(),
            stack: Some("Python".into()),
            startup_command: None,
        });
    }
    if activities.is_empty() {
        if let Some(a) = from_makefile(project, read_file) {
            activities.push(a);
        }
    }
    activities
}

fn from_package_json(
    project: &Project,
    read_file: &dyn Fn(&str) -> Option<String>,
) -> Option<Activity> {
    let text = read_file(&format!("{}/package.json", project.path))?;
    let pkg: PackageJson = serde_json::from_str(&text).ok()?;
    let scripts = pkg.scripts.unwrap_or_default();
    let dev_script = DEV_SCRIPT_NAMES
        .iter()
        .find(|name| scripts.contains_key(**name))?;
    let pm = package_manager(project);
    Some(Activity {
        name: format!("{} Development", title_case(&project.name)),
        environment: None,
        workspace: project.path.clone(),
        stack: Some("Node.js".into()),
        startup_command: Some(format!("{pm} {dev_script}")),
    })
}

/// Detect the package manager from lockfiles/scripts convention.
fn package_manager(project: &Project) -> &'static str {
    if project.markers.iter().any(|m| m == "pnpm-workspace.yaml") {
        "pnpm"
    } else {
        // Without stronger evidence, `npm run` works everywhere.
        "npm run"
    }
}

fn from_cargo(project: &Project, read_file: &dyn Fn(&str) -> Option<String>) -> Activity {
    let has_bin = read_file(&format!("{}/Cargo.toml", project.path))
        .map(|t| t.contains("[[bin]]") || t.contains("[package]"))
        .unwrap_or(true);
    Activity {
        name: format!("{} Development", title_case(&project.name)),
        environment: None,
        workspace: project.path.clone(),
        stack: Some("Rust".into()),
        startup_command: has_bin.then(|| "cargo run".to_string()),
    }
}

fn from_go(project: &Project) -> Activity {
    Activity {
        name: format!("{} Development", title_case(&project.name)),
        environment: None,
        workspace: project.path.clone(),
        stack: Some("Go".into()),
        startup_command: Some("go run .".into()),
    }
}

fn from_makefile(
    project: &Project,
    read_file: &dyn Fn(&str) -> Option<String>,
) -> Option<Activity> {
    let text = read_file(&format!("{}/Makefile", project.path))?;
    for target in ["dev", "run", "serve", "start"] {
        let prefix = format!("{target}:");
        if text.lines().any(|l| l.starts_with(&prefix)) {
            return Some(Activity {
                name: format!("{} Development", title_case(&project.name)),
                environment: None,
                workspace: project.path.clone(),
                stack: None,
                startup_command: Some(format!("make {target}")),
            });
        }
    }
    None
}

/// "api-server" -> "Api Server", "frontend" -> "Frontend".
pub fn title_case(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn project(markers: &[&str]) -> Project {
        Project {
            path: "/home/dev/api".into(),
            name: "api".into(),
            markers: markers.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn reader<'a>(files: HashMap<&'a str, &'a str>) -> impl Fn(&str) -> Option<String> + use<'a> {
        move |path| files.get(path).map(|s| s.to_string())
    }

    #[test]
    fn infers_node_dev_activity_from_scripts() {
        let files = HashMap::from([(
            "/home/dev/api/package.json",
            r#"{"name":"api","scripts":{"dev":"vite","build":"vite build"}}"#,
        )]);
        let acts = infer_activities(&project(&["package.json"]), &reader(files));
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].name, "Api Development");
        assert_eq!(acts[0].startup_command.as_deref(), Some("npm run dev"));
        assert_eq!(acts[0].stack.as_deref(), Some("Node.js"));
    }

    #[test]
    fn prefers_pnpm_for_pnpm_workspaces() {
        let files = HashMap::from([(
            "/home/dev/api/package.json",
            r#"{"scripts":{"dev":"node server.js"}}"#,
        )]);
        let acts = infer_activities(
            &project(&["package.json", "pnpm-workspace.yaml"]),
            &reader(files),
        );
        assert_eq!(acts[0].startup_command.as_deref(), Some("pnpm dev"));
    }

    #[test]
    fn no_dev_script_means_no_node_activity() {
        let files = HashMap::from([(
            "/home/dev/api/package.json",
            r#"{"scripts":{"build":"tsc"}}"#,
        )]);
        let acts = infer_activities(&project(&["package.json"]), &reader(files));
        assert!(acts.is_empty());
    }

    #[test]
    fn infers_rust_and_go_activities() {
        let files = HashMap::from([("/home/dev/api/Cargo.toml", "[package]\nname=\"api\"")]);
        let rust = infer_activities(&project(&["Cargo.toml"]), &reader(files));
        assert_eq!(rust[0].startup_command.as_deref(), Some("cargo run"));

        let go = infer_activities(&project(&["go.mod"]), &reader(HashMap::new()));
        assert_eq!(go[0].startup_command.as_deref(), Some("go run ."));
    }

    #[test]
    fn falls_back_to_makefile_targets() {
        let files = HashMap::from([("/home/dev/api/Makefile", "dev:\n\t./run.sh\n")]);
        let acts = infer_activities(&project(&["Makefile"]), &reader(files));
        assert_eq!(acts[0].startup_command.as_deref(), Some("make dev"));
    }

    #[test]
    fn title_cases_names() {
        assert_eq!(title_case("api-server"), "Api Server");
        assert_eq!(title_case("frontend"), "Frontend");
    }
}

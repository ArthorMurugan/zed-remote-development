//! Shared data model for zrd and its stdio protocol.
//!
//! These types are serialized as JSON and consumed by the Zed extension,
//! so field names here are part of the wire contract between zrd and the
//! extension.

use crate::ssh::SshTarget;
use serde::{Deserialize, Serialize};

/// A machine the user can connect to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Machine {
    pub name: String,
    pub target: SshTarget,
    /// Where this machine came from: `zrd-config` or `ssh-config`.
    pub source: String,
}

/// What kind of implementation backs a development environment.
/// Deliberately hidden from primary UI copy: users see
/// "Development Environment", not "container 4f7a8c".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentKind {
    Docker,
    DevContainer,
    Compose,
}

impl std::fmt::Display for EnvironmentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvironmentKind::Docker => write!(f, "Docker"),
            EnvironmentKind::DevContainer => write!(f, "Dev Container"),
            EnvironmentKind::Compose => write!(f, "Compose"),
        }
    }
}

/// Explicit lifecycle state for an environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentState {
    Unknown,
    Stopped,
    Starting,
    Running,
    Building,
    Rebuilding,
    Failed,
}

impl std::fmt::Display for EnvironmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvironmentState::Unknown => write!(f, "unknown"),
            EnvironmentState::Stopped => write!(f, "stopped"),
            EnvironmentState::Starting => write!(f, "starting"),
            EnvironmentState::Running => write!(f, "running"),
            EnvironmentState::Building => write!(f, "building"),
            EnvironmentState::Rebuilding => write!(f, "rebuilding"),
            EnvironmentState::Failed => write!(f, "failed"),
        }
    }
}

/// A development environment discovered on a machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    /// Human name: `api`, `frontend`, `devcontainer` — never a container ID.
    pub name: String,
    pub kind: EnvironmentKind,
    pub state: EnvironmentState,
    /// Short description for the picker ("Node.js API", image name, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Host-side project path associated with this environment, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    /// Resolved host-side workspace path Zed should open, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Implementation detail kept for lifecycle operations (container name,
    /// compose service, ...). Never the primary label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

/// A project discovered on a machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub path: String,
    pub name: String,
    /// Markers found: `.git`, `package.json`, `Cargo.toml`, ...
    pub markers: Vec<String>,
}

/// A higher-level "Development Activity": environment + workspace +
/// startup command, inferred from project metadata where possible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    /// e.g. "API Development", "Frontend Development".
    pub name: String,
    /// Name of the environment this activity runs in, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// Host-side workspace path.
    pub workspace: String,
    /// e.g. "Node.js", "Rust", "Go" — inferred stack hint for the picker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    /// Startup command such as `pnpm dev`, if one could be inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_command: Option<String>,
}

/// Everything discovered about a machine in one pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub machine: String,
    pub environments: Vec<Environment>,
    pub projects: Vec<Project>,
    pub activities: Vec<Activity>,
    /// Ordered, human-readable progress lines
    /// ("Connected", "Docker detected", ...).
    pub progress: Vec<String>,
}

/// A remembered session for trivial reconnects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentSession {
    pub machine: SshTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    pub workspace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    pub last_opened_at: String,
}

/// The result of an `open` operation: everything the caller needs to
/// understand what happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenResult {
    pub machine: SshTarget,
    pub environment: Option<Environment>,
    pub workspace: String,
    /// The URL handed to Zed, e.g. `ssh://dev@host/workspace/api`.
    pub zed_url: String,
    pub progress: Vec<String>,
}

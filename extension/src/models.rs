//! Wire types mirroring zrd's JSON output.
//!
//! Field names must stay in sync with `zrd/src/models.rs`.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Machine {
    pub name: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentKind {
    Docker,
    DevContainer,
    Compose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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
        write!(f, "{}", format!("{self:?}").to_ascii_lowercase())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Environment {
    pub name: String,
    pub kind: EnvironmentKind,
    pub state: EnvironmentState,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub markers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Activity {
    pub name: String,
    #[serde(default)]
    pub environment: Option<String>,
    pub workspace: String,
    #[serde(default)]
    pub stack: Option<String>,
    #[serde(default)]
    pub startup_command: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    #[serde(default)]
    pub environments: Vec<Environment>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub activities: Vec<Activity>,
    #[serde(default)]
    pub progress: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecentSession {
    #[serde(default)]
    pub environment: Option<String>,
    pub workspace: String,
    #[serde(default)]
    pub activity: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenResult {
    pub environment: Option<Environment>,
    pub workspace: String,
    pub zed_url: String,
    #[serde(default)]
    pub progress: Vec<String>,
}

//! Machine registry and recent-session persistence.
//!
//! State lives in `$XDG_CONFIG_HOME/zrd/` (or `~/.config/zrd/`):
//!   - `machines.toml` — machines added through `zrd add-machine`
//!   - `state.json`    — recent sessions for one-click reconnects
//!
//! `ZRD_CONFIG_DIR` overrides the directory (used by tests).

use crate::models::RecentSession;
use crate::ssh::SshTarget;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MachineRegistry {
    #[serde(default)]
    pub machines: Vec<RegisteredMachine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredMachine {
    pub name: String,
    pub target: SshTarget,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub recent: Vec<RecentSession>,
}

pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ZRD_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zrd")
}

impl MachineRegistry {
    pub fn load() -> Result<Self> {
        let path = config_dir().join("machines.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = toml::to_string_pretty(self)?;
        std::fs::write(dir.join("machines.toml"), text)?;
        Ok(())
    }

    pub fn add(&mut self, name: String, target: SshTarget) {
        // Replace an existing registration of the same name.
        self.machines.retain(|m| m.name != name);
        self.machines.push(RegisteredMachine { name, target });
    }

    pub fn find(&self, name_or_host: &str) -> Option<SshTarget> {
        self.machines
            .iter()
            .find(|m| m.name == name_or_host || m.target.host == name_or_host || m.target.authority() == name_or_host)
            .map(|m| m.target.clone())
    }
}

impl State {
    pub fn load() -> Result<Self> {
        let path = config_dir().join("state.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(dir.join("state.json"), text)?;
        Ok(())
    }

    /// Record a session, most recent first, deduplicated by
    /// (machine, environment, workspace), capped at 20 entries.
    pub fn remember(&mut self, session: RecentSession) {
        self.recent.retain(|s| {
            !(s.machine.authority() == session.machine.authority()
                && s.environment == session.environment
                && s.workspace == session.workspace)
        });
        self.recent.insert(0, session);
        self.recent.truncate(20);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_add_replaces_same_name() {
        let mut reg = MachineRegistry::default();
        let t1 = SshTarget::parse("dev@old.example.com").unwrap();
        let t2 = SshTarget::parse("dev@new.example.com").unwrap();
        reg.add("dev".into(), t1);
        reg.add("dev".into(), t2);
        assert_eq!(reg.machines.len(), 1);
        assert_eq!(reg.find("dev").unwrap().host, "new.example.com");
    }

    #[test]
    fn state_deduplicates_recent_sessions() {
        let mut state = State::default();
        let t = SshTarget::parse("dev@host").unwrap();
        let s = RecentSession {
            machine: t.clone(),
            environment: Some("api".into()),
            workspace: "/workspace/api".into(),
            activity: None,
            last_opened_at: "2026-01-01T00:00:00Z".into(),
        };
        state.remember(s.clone());
        state.remember(s);
        assert_eq!(state.recent.len(), 1);
    }
}

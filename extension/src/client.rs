//! Client for the `zrd` native agent.
//!
//! The extension runs inside Zed's WASM sandbox; every operation is
//! delegated to the `zrd` binary on PATH (requires the `process:exec`
//! capability declared in extension.toml). All responses are JSON.

use crate::models::{Discovery, Machine, OpenResult, RecentSession};
use zed_extension_api as zed;

#[derive(Debug, Clone)]
pub struct ZrdClient {
    binary: String,
}

impl Default for ZrdClient {
    fn default() -> Self {
        Self {
            binary: "zrd".to_string(),
        }
    }
}

impl ZrdClient {
    pub fn new(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    fn run_json<T: serde::de::DeserializeOwned>(&self, args: &[&str]) -> Result<T, String> {
        let output = zed::process::Command::new(&self.binary)
            .args(args.iter().map(|s| s.to_string()))
            .output()
            .map_err(|e| {
                format!(
                    "Could not run the zrd agent ({e}).\n\n\
                     Install it with `cargo install --path zrd` from the \
                     zed-remote-development repository and make sure `zrd` is on PATH."
                )
            })?;
        if output.status != Some(0) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // zrd already writes friendly, user-first error messages.
            return Err(stderr.trim().to_string());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout).map_err(|e| format!("unexpected zrd output: {e}"))
    }

    /// Verify the agent is installed and get its version.
    pub fn ping(&self) -> Result<String, String> {
        let output = zed::process::Command::new(&self.binary)
            .arg("--version")
            .output()
            .map_err(|e| format!("zrd is not available: {e}"))?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn machines(&self) -> Result<Vec<Machine>, String> {
        self.run_json(&["machines", "--json"])
    }

    pub fn recent(&self) -> Result<Vec<RecentSession>, String> {
        self.run_json(&["recent", "--json"])
    }

    pub fn add_machine(&self, address: &str) -> Result<(), String> {
        self.run_json::<serde_json::Value>(&["add-machine", address])
            .map(|_| ())
    }

    pub fn discover(&self, machine: &str) -> Result<Discovery, String> {
        self.run_json(&["discover", machine, "--json"])
    }

    pub fn open(&self, machine: &str, environment: Option<&str>) -> Result<OpenResult, String> {
        self.open_opts(machine, environment, false)
    }

    pub fn open_opts(
        &self,
        machine: &str,
        environment: Option<&str>,
        no_launch: bool,
    ) -> Result<OpenResult, String> {
        let mut args = vec!["open", machine, "--json"];
        if let Some(env) = environment {
            args.push("--env");
            args.push(env);
        }
        if no_launch {
            args.push("--no-launch");
        }
        self.run_json(&args)
    }

    pub fn reopen(&self, environment: Option<&str>) -> Result<OpenResult, String> {
        self.reopen_opts(environment, false)
    }

    pub fn reopen_opts(
        &self,
        environment: Option<&str>,
        no_launch: bool,
    ) -> Result<OpenResult, String> {
        let mut args = vec!["reopen", "--json"];
        if let Some(env) = environment {
            args.push("--env");
            args.push(env);
        }
        if no_launch {
            args.push("--no-launch");
        }
        self.run_json(&args)
    }

    pub fn rebuild(&self, machine: &str, environment: &str) -> Result<(), String> {
        self.run_json::<serde_json::Value>(&["rebuild", machine, "--env", environment])
            .map(|_| ())
    }

    pub fn logs(&self, machine: &str, environment: &str) -> Result<String, String> {
        let output = zed::process::Command::new(&self.binary)
            .args(["logs", machine, "--env", environment])
            .output()
            .map_err(|e| format!("zrd is not available: {e}"))?;
        if output.status != Some(0) {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    pub fn diagnose(&self, machine: &str) -> Result<String, String> {
        let output = zed::process::Command::new(&self.binary)
            .args(["diagnose", machine])
            .output()
            .map_err(|e| format!("zrd is not available: {e}"))?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Hand-off: ask zrd to launch Zed on a remote URL.
    ///
    /// Routed through zrd because the extension API cannot open projects,
    /// and the `process:exec` capability is scoped to the zrd binary only.
    pub fn launch_zed(&self, ssh_url: &str) -> Result<(), String> {
        zed::process::Command::new(&self.binary)
            .args(["launch", ssh_url])
            .output()
            .map_err(|e| format!("could not open the project in Zed: {e}"))?;
        Ok(())
    }
}

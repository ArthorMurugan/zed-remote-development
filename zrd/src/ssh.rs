//! SSH target parsing, `~/.ssh/config` discovery, and remote command execution.
//!
//! zrd never implements the SSH protocol itself. It orchestrates the user's
//! existing `ssh` binary, exactly like Zed does, so `~/.ssh/config`, ssh-agent,
//! ControlMaster and jump hosts all keep working without configuration.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Environment variable that overrides the ssh binary used for remote command
/// execution. Primarily used by tests to substitute a fake remote.
pub const SSH_BIN_ENV: &str = "ZRD_SSH_BIN";

/// A parsed SSH target such as `user@host:2222`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTarget {
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Optional display name shown in pickers ("Development Server").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional extra arguments passed verbatim to ssh (identity files,
    /// jump hosts, ...). Kept out of the primary UI on purpose.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
}

impl SshTarget {
    /// Parse `user@host`, `host`, `user@host:port` or `host:port`.
    ///
    /// An optional `ssh://` prefix is accepted so CLI URLs can be reused.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Err(anyhow!("empty SSH target"));
        }
        let input = input.strip_prefix("ssh://").unwrap_or(input);
        // Drop any path component: zrd resolves workspaces itself.
        //
        // Three forms exist:
        //   ssh://user@host:2222/abs/path   (authority + '/' + path)
        //   host:~/project  or  host:/abs   (scp-style, path after ':')
        //   user@host:2222                  (no path)
        let scp_cut = input.find(":~").or_else(|| input.find(":/"));
        let authority = if let Some(idx) = scp_cut {
            &input[..idx]
        } else {
            match input.find('/') {
                Some(idx) => &input[..idx],
                None => input,
            }
        };
        if authority.is_empty() {
            return Err(anyhow!("invalid SSH target: {input}"));
        }

        let (user, hostport) = match authority.split_once('@') {
            Some((u, rest)) => {
                if u.is_empty() {
                    return Err(anyhow!("invalid SSH target (empty user): {input}"));
                }
                (Some(u.to_string()), rest)
            }
            None => (None, authority),
        };

        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() => {
                let port: u16 = p
                    .parse()
                    .with_context(|| format!("invalid port in SSH target: {input}"))?;
                (h.to_string(), Some(port))
            }
            _ => (hostport.to_string(), None),
        };

        if host.is_empty() {
            return Err(anyhow!("invalid SSH target (empty host): {input}"));
        }
        Ok(SshTarget {
            host,
            user,
            port,
            name: None,
            extra_args: Vec::new(),
        })
    }

    /// `user@host` (user omitted when not set) — used for display and URLs.
    pub fn authority(&self) -> String {
        match &self.user {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }

    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.authority())
    }
}

impl std::fmt::Display for SshTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.authority())?;
        if let Some(p) = self.port {
            write!(f, ":{p}")?;
        }
        Ok(())
    }
}

/// A single host entry discovered in `~/.ssh/config`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SshConfigHost {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
}

/// Minimal parser for `~/.ssh/config`.
///
/// The user's ssh config is treated as the source of truth: hosts defined
/// there are offered as machines, and per-host options are left to ssh
/// itself (we only surface them for display and machine import).
pub struct SshConfig {
    pub hosts: Vec<SshConfigHost>,
}

impl SshConfig {
    pub fn load() -> Result<Self> {
        let path = default_ssh_config_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(SshConfig { hosts: Vec::new() });
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Ok(Self::parse(&text))
    }

    pub fn parse(text: &str) -> Self {
        let mut hosts = Vec::new();
        let mut current: Option<SshConfigHost> = None;

        for raw_line in text.lines() {
            // Strip comments and whitespace.
            let line = match raw_line.find('#') {
                Some(idx) => &raw_line[..idx],
                None => raw_line,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, char::is_whitespace);
            let keyword = parts.next().unwrap_or("").to_ascii_lowercase();
            let value = parts.next().unwrap_or("").trim().to_string();

            if keyword == "host" {
                if let Some(h) = current.take() {
                    hosts.push(h);
                }
                current = Some(SshConfigHost {
                    pattern: value,
                    ..Default::default()
                });
                continue;
            }
            let Some(entry) = current.as_mut() else {
                continue; // global options before any Host block are ignored
            };
            match keyword.as_str() {
                "hostname" => entry.host_name = Some(value),
                "user" => entry.user = Some(value),
                "port" => entry.port = value.parse().ok(),
                "identityfile" => entry.identity_file = Some(value),
                "proxyjump" => entry.proxy_jump = Some(value),
                _ => {}
            }
        }
        if let Some(h) = current.take() {
            hosts.push(h);
        }
        // Wildcard-only blocks (`Host *`) are defaults, not machines.
        let hosts = hosts
            .into_iter()
            .filter(|h| !h.pattern.contains('*') && !h.pattern.contains('?') && !h.pattern.contains('!'))
            .collect();
        SshConfig { hosts }
    }

    /// Convert a config host entry into a connectable target.
    pub fn to_target(entry: &SshConfigHost) -> SshTarget {
        SshTarget {
            host: entry.host_name.clone().unwrap_or_else(|| entry.pattern.clone()),
            user: entry.user.clone(),
            port: entry.port,
            name: Some(entry.pattern.clone()),
            // ssh resolves IdentityFile/ProxyJump itself from the config
            // when we connect by pattern, so no extra args are needed.
            extra_args: Vec::new(),
        }
    }

    /// All non-wildcard host entries, as connectable targets keyed by alias.
    pub fn targets(&self) -> Vec<SshTarget> {
        self.hosts.iter().map(Self::to_target).collect()
    }
}

pub fn default_ssh_config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("ZRD_SSH_CONFIG") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ssh")
        .join("config")
}

/// Executes commands on a remote machine through the user's `ssh` binary.
#[derive(Debug, Clone)]
pub struct RemoteShell {
    target: SshTarget,
}

impl RemoteShell {
    pub fn new(target: SshTarget) -> Self {
        Self { target }
    }

    pub fn target(&self) -> &SshTarget {
        &self.target
    }

    fn ssh_bin() -> String {
        std::env::var(SSH_BIN_ENV).unwrap_or_else(|_| "ssh".to_string())
    }

    /// The exact ssh invocation, for diagnostics. Never shown as a primary
    /// error message.
    pub fn describe_command(&self, remote_command: &str) -> String {
        format!(
            "{} {} {}",
            Self::ssh_bin(),
            self.base_args().join(" "),
            remote_command
        )
    }

    fn base_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        // Non-interactive: fail fast instead of hanging on a password prompt.
        // Zed's own connection will handle interactive prompts when it opens.
        args.push("-o".into());
        args.push("BatchMode=yes".into());
        args.push("-o".into());
        args.push("ConnectTimeout=10".into());
        if let Some(port) = self.target.port {
            args.push("-p".into());
            args.push(port.to_string());
        }
        args.extend(self.target.extra_args.iter().cloned());
        args.push(self.target.authority());
        args
    }

    /// Run a shell command on the remote machine and return stdout.
    pub fn run(&self, remote_command: &str) -> Result<String> {
        let output = Command::new(Self::ssh_bin())
            .args(self.base_args())
            .arg(remote_command)
            .output()
            .with_context(|| {
                format!(
                    "failed to execute ssh — is an ssh client installed and on PATH?"
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "remote command failed ({}): {}",
                output.status,
                stderr.trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Run a command, returning `Ok(None)` instead of an error when the
    /// remote side reports "not found" style failures. Used for optional
    /// probes (docker presence, devcontainer CLI, ...).
    pub fn probe(&self, remote_command: &str) -> Option<String> {
        self.run(remote_command).ok()
    }

    /// Check connectivity. Returns a short description of the remote host.
    pub fn check_connection(&self) -> Result<String> {
        let uname = self
            .run("uname -srm 2>/dev/null || echo unknown")
            .context("could not reach the machine over SSH")?;
        Ok(uname.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_host() {
        let t = SshTarget::parse("server.example.com").unwrap();
        assert_eq!(t.host, "server.example.com");
        assert_eq!(t.user, None);
        assert_eq!(t.port, None);
    }

    #[test]
    fn parses_user_at_host() {
        let t = SshTarget::parse("dev@server.example.com").unwrap();
        assert_eq!(t.host, "server.example.com");
        assert_eq!(t.user.as_deref(), Some("dev"));
        assert_eq!(t.port, None);
    }

    #[test]
    fn parses_user_host_port() {
        let t = SshTarget::parse("dev@server.example.com:2222").unwrap();
        assert_eq!(t.host, "server.example.com");
        assert_eq!(t.user.as_deref(), Some("dev"));
        assert_eq!(t.port, Some(2222));
    }

    #[test]
    fn parses_ssh_url_and_drops_path() {
        let t = SshTarget::parse("ssh://dev@server.example.com:2222/home/dev/project").unwrap();
        assert_eq!(t.host, "server.example.com");
        assert_eq!(t.port, Some(2222));
        assert_eq!(t.user.as_deref(), Some("dev"));
    }

    #[test]
    fn parses_scp_style_and_drops_path() {
        let t = SshTarget::parse("dev@server.example.com:~/project").unwrap();
        assert_eq!(t.host, "server.example.com");
        assert_eq!(t.port, None);
    }

    #[test]
    fn rejects_empty_and_bad_port() {
        assert!(SshTarget::parse("").is_err());
        assert!(SshTarget::parse("@host").is_err());
        assert!(SshTarget::parse("host:notaport/").is_err());
    }

    #[test]
    fn authority_roundtrip() {
        let t = SshTarget::parse("dev@gpu.internal:2222").unwrap();
        assert_eq!(t.authority(), "dev@gpu.internal");
        assert_eq!(t.to_string(), "dev@gpu.internal:2222");
    }

    #[test]
    fn parses_ssh_config_hosts() {
        let text = r#"
Host *
    ServerAliveInterval 30

Host dev-server
    HostName 192.168.1.10
    User dev
    Port 2222
    IdentityFile ~/.ssh/work_id

Host gpu
    HostName gpu.internal
    User researcher
    ProxyJump bastion

Host prod-* 
    User ops
"#;
        let cfg = SshConfig::parse(text);
        // Wildcard blocks must not become machines.
        assert_eq!(cfg.hosts.len(), 2);
        let dev = &cfg.hosts[0];
        assert_eq!(dev.pattern, "dev-server");
        assert_eq!(dev.host_name.as_deref(), Some("192.168.1.10"));
        assert_eq!(dev.user.as_deref(), Some("dev"));
        assert_eq!(dev.port, Some(2222));
        assert_eq!(dev.identity_file.as_deref(), Some("~/.ssh/work_id"));

        let gpu = &cfg.hosts[1];
        assert_eq!(gpu.proxy_jump.as_deref(), Some("bastion"));

        let targets = cfg.targets();
        assert_eq!(targets[0].host, "192.168.1.10");
        assert_eq!(targets[0].name.as_deref(), Some("dev-server"));
        assert_eq!(targets[1].host, "gpu.internal");
    }

    #[test]
    fn handles_config_without_host_blocks() {
        let cfg = SshConfig::parse("ServerAliveInterval 30\n# only globals\n");
        assert!(cfg.hosts.is_empty());
    }
}

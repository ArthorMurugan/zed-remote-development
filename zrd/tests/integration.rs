//! Integration test: connect → discover → start → open, against a fake
//! remote machine implemented as a shell script standing in for `ssh`.
//!
//! No real SSH server or Docker daemon is required: zrd's ssh layer is
//! pointed at the fake via ZRD_SSH_BIN, and Zed at a recorder script via
//! ZRD_ZED_BIN.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn write_executable(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    f.set_permissions(std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A fake remote machine with:
///   - two Docker containers: `api` (stopped, mounts /home/dev/api at
///     /workspace/api) and `postgres` (running)
///   - a project at /home/dev/api with package.json and a devcontainer
const FAKE_SSH: &str = r#"#!/bin/sh
# The remote command is always the last argument.
# The remote command is always the last argument (POSIX-sh safe).
for last; do :; done; cmd="$last"

case "$cmd" in
  *"command -v docker"*)
    echo ok ;;
  *"docker ps -a --format"*)
    echo '{"ID":"4f7a8c","Names":"api","Image":"node:20","State":"exited","Status":"Exited (0) 3 days ago","Labels":"","Mounts":""}'
    echo '{"ID":"9aa1","Names":"postgres","Image":"postgres:16","State":"running","Status":"Up 2 hours","Labels":"","Mounts":""}'
    ;;
  *"docker inspect 'api'"*)
    cat <<'EOF'
[{"Name":"/api","Config":{"Image":"node:20","WorkingDir":"/workspace/api","Labels":{}},"State":{"Status":"exited","Running":false},"Mounts":[{"Source":"/home/dev/api","Destination":"/workspace/api"}]}]
EOF
    ;;
  *"docker inspect 'postgres'"*)
    cat <<'EOF'
[{"Name":"/postgres","Config":{"Image":"postgres:16","WorkingDir":"","Labels":{}},"State":{"Status":"running","Running":true},"Mounts":[]}]
EOF
    ;;
  *"docker start"*)
    echo api ;;
  *"command -v devcontainer"*)
    echo ok ;;
  *"devcontainer up"*)
    echo '{"outcome":"success"}' ;;
  *"cat '/home/dev/api/.devcontainer/devcontainer.json'"*)
    cat <<'EOF'
{ "name": "api", "image": "node:20", "workspaceFolder": "/workspace/api" }
EOF
    ;;
  *"cat '/home/dev/api/.devcontainer.json'"*)
    : ;; # not present
  *"test -f '/home/dev/api/compose.yml'"*|*"test -f '/home/dev/api/compose.yaml'"*|*"test -f '/home/dev/api/docker-compose.yml'"*|*"test -f '/home/dev/api/docker-compose.yaml'"*)
    : ;; # no compose file in this fixture
  *"find"*)
    echo "/home/dev/api/.git"
    echo "/home/dev/api/package.json"
    echo "/home/dev/api/.devcontainer/devcontainer.json"
    ;;
  *"uname"*)
    echo "Linux devbox 6.8.0 x86_64" ;;
  *"echo ok"*)
    echo ok ;;
  *)
    : ;;
esac
exit 0
"#;

const FAKE_ZED: &str = r#"#!/bin/sh
echo "$@" >> "$ZRD_TEST_ZED_LOG"
"#;

#[test]
fn connect_discover_start_open_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let fake_ssh = write_executable(tmp.path(), "fake-ssh.sh", FAKE_SSH);
    let zed_log = tmp.path().join("zed.log");
    let fake_zed = write_executable(tmp.path(), "fake-zed.sh", FAKE_ZED);

    let zrd = env!("CARGO_BIN_EXE_zrd");
    let output = Command::new(zrd)
        .args(["open", "dev@fakehost", "--env", "api", "--json"])
        .env("ZRD_SSH_BIN", &fake_ssh)
        .env("ZRD_ZED_BIN", &fake_zed)
        .env("ZRD_CONFIG_DIR", tmp.path().join("config"))
        .env("ZRD_TEST_ZED_LOG", &zed_log)
        .env("ZRD_SSH_CONFIG", tmp.path().join("no-such-config"))
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "zrd open failed\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // The environment was resolved to a human name, never a container ID.
    assert_eq!(result["environment"]["name"], "api");
    // Workspace mapped back to the host-side path.
    assert_eq!(result["workspace"], "/home/dev/api");
    // Documented Zed CLI URL generated for the hand-off.
    assert_eq!(result["zed_url"], "ssh://dev@fakehost/home/dev/api");

    let progress: Vec<&str> = result["progress"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(progress.iter().any(|l| l.contains("Connected")));
    assert!(progress.iter().any(|l| l.contains("Docker detected")));
    assert!(progress.iter().any(|l| l.contains("Starting 'api'")));

    // Zed was actually launched with the ssh:// URL.
    let launched = std::fs::read_to_string(&zed_log).unwrap();
    assert!(launched.contains("ssh://dev@fakehost/home/dev/api"));

    // The session was remembered for reconnects.
    let recent = Command::new(zrd)
        .args(["recent", "--json"])
        .env("ZRD_CONFIG_DIR", tmp.path().join("config"))
        .output()
        .unwrap();
    let recent: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&recent.stdout)).unwrap();
    assert_eq!(recent[0]["environment"], "api");
    assert_eq!(recent[0]["workspace"], "/home/dev/api");
}

#[test]
fn discover_lists_environments_with_human_names() {
    let tmp = tempfile::tempdir().unwrap();
    let fake_ssh = write_executable(tmp.path(), "fake-ssh.sh", FAKE_SSH);

    let zrd = env!("CARGO_BIN_EXE_zrd");
    let output = Command::new(zrd)
        .args(["discover", "dev@fakehost", "--json"])
        .env("ZRD_SSH_BIN", &fake_ssh)
        .env("ZRD_CONFIG_DIR", tmp.path().join("config"))
        .env("ZRD_SSH_CONFIG", tmp.path().join("no-such-config"))
        .output()
        .unwrap();

    assert!(output.status.success());
    let d: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let names: Vec<&str> = d["environments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"api"));
    assert!(names.contains(&"postgres"));
    assert!(!names.iter().any(|n| n.contains("4f7a8c")));
}

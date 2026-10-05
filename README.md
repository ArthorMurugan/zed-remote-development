# Zed Remote Development

A Zed dev extension and native `zrd` agent for discovering remote development
environments over SSH and opening them in Zed. The extension provides slash
commands; the native agent handles SSH, environment discovery, and the handoff
to Zed's remote workspace support.

## Requirements

- Zed with remote SSH support.
- Rust and Cargo (installable through [rustup](https://rustup.rs/)).
- SSH client access to the remote machine, configured either in
  `~/.ssh/config` or with a `user@host` address.
- Docker available on the remote machine for container-backed environments.
- The Zed CLI available as `zed` on `PATH` when opening remote projects. On
  macOS, install it from Zed's command palette with `cli: install cli binary`.

## Install from a GitHub Release (Linux x86_64)

Push a version tag such as `v0.1.0` to build the Linux `zrd` executable and
the Zed extension WASM bundle and attach them to a GitHub Release:

```sh
git tag v0.1.0
git push origin v0.1.0
```

After the release workflow completes, install the latest release:

```sh
curl -fsSL https://github.com/ArthorMurugan/zed-remote-development/releases/latest/download/install-linux.sh | bash
```

The script installs or updates `zrd` in `~/.local/bin` and stages the extension
package (including its prebuilt WASM) under `~/.local/share/zrd/extension`.
Ensure `~/.local/bin` is on `PATH` and restart Zed if it was already running.

Because this extension is not in Zed's registry, activate it once as a dev
extension:

1. Open Zed's Extensions view and choose **Install Dev Extension** (or run
   `zed: install dev extension` from the command palette).
2. Select the staged `extension/` directory printed by the installer. Zed may
   compile the extension from its included Rust source when installing it.

Rerun the installer to update both release files. To uninstall, first remove
the dev extension in Zed's Extensions view, then run:

```sh
curl -fsSL https://github.com/ArthorMurugan/zed-remote-development/releases/latest/download/uninstall-linux.sh | bash
```

Uninstallation removes the binary and staged extension files, but preserves
your `~/.config/zrd/` machine registrations and session state.

## Build from source

For local development, run `make setup` to build the extension WASM and install
the native agent through Cargo. Install the `extension/` directory as a dev
extension in Zed. Use `make wasm` to build only the WASM bundle.

Useful build and test targets:

| Command | Action |
| --- | --- |
| `make` or `make all` | Build the WASM extension and native agent |
| `make wasm` | Build the WASM extension |
| `make agent` | Build the native `zrd` executable |
| `make install` | Install or update the latest GitHub Release |
| `make uninstall` | Remove release-installed files |
| `make install-release` | Install or update the latest GitHub Release |
| `make uninstall-release` | Remove release-installed files |
| `make install-agent` | Install or update `zrd` in Cargo's binary directory |
| `make setup` | Build the WASM extension and install the native agent |
| `make test` | Run the native agent's tests |

## Connect to a machine

You can use an existing SSH alias. For example, add a host to `~/.ssh/config`:

```sshconfig
Host devbox
    HostName devbox.example.com
    User developer
    IdentityFile ~/.ssh/id_ed25519
```

Alternatively, register a machine from Zed with `/remote-add developer@devbox.example.com`.
Registration is also available from a terminal:

```sh
zrd add-machine developer@devbox.example.com
```

The extension also discovers non-wildcard host aliases in `~/.ssh/config`.
Check connectivity and remote prerequisites with `/remote-diagnose` followed
by a machine name, or run `zrd diagnose devbox` in a terminal.

## Use the Zed slash commands

Open a Zed editor or assistant input and enter `/` to find the extension's
commands. Start with `/remote-connect`; argument completions offer available
machines and environments.

| Command | Purpose | Example |
| --- | --- | --- |
| `/remote-connect [machine]` | List machines, or connect and discover environments | `/remote-connect devbox` |
| `/remote-open <machine> [environment]` | Start/select an environment and open its workspace in Zed | `/remote-open devbox api` |
| `/remote-reopen [environment]` | Reopen the most recent remote session | `/remote-reopen` |
| `/remote-add <user@host>` | Register a machine | `/remote-add developer@devbox.example.com` |
| `/remote-logs <machine> <environment>` | Show recent environment logs | `/remote-logs devbox api` |
| `/remote-diagnose <machine>` | Check SSH, Docker, and Zed connectivity | `/remote-diagnose devbox` |

`/remote-open` can omit the environment when discovery finds only one suitable
choice. If there are multiple environments, use the argument completions to
select one. When discovery finds projects but no container environments, open
the project directly using the path reported by `/remote-connect` and the
native CLI:

```sh
zrd open devbox --project /path/to/project
```

## Native agent configuration

The agent uses your existing SSH client configuration and credentials. Machine
registrations and recent sessions are stored under `~/.config/zrd/` (or the
platform's equivalent configuration directory):

- `machines.toml` — machines added with `/remote-add` or `zrd add-machine`.
- `state.json` — recent sessions for `/remote-reopen`.

Advanced SSH settings can be supplied in `~/.ssh/config` or through the agent:

```sh
zrd add-machine developer@devbox.example.com --port 2222
zrd add-machine developer@devbox.example.com --identity ~/.ssh/devbox_key
zrd add-machine developer@devbox.example.com --jump bastion
```

The `zrd` executable must be discoverable on the `PATH` used by Zed. If Zed
reports that it cannot run the agent, check `command -v zrd` in a terminal,
ensure Cargo's binary directory is on `PATH`, then restart Zed.

## Troubleshooting

- **No machines appear:** check `~/.ssh/config` or register an address with
  `/remote-add user@host`.
- **SSH or environment discovery fails:** run `/remote-diagnose <machine>` and
  confirm that SSH authentication works from a terminal.
- **Zed does not open the remote workspace:** make sure the Zed CLI is
  installed and available on `PATH`; test the `zed` command in a terminal.
- **The extension cannot start `zrd`:** make sure `~/.local/bin` (for a
  release install) or `~/.cargo/bin` (for a source install) is on `PATH`, then
  restart Zed.
- **The dev extension does not load:** install the `extension/` directory,
  not the repository root, and inspect Zed's log from the command palette.

This repository currently documents local dev-extension installation. Before
publishing it to the Zed extension registry, set the real repository URL in
`extension/extension.toml` and follow Zed's registry submission requirements.

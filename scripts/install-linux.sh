#!/usr/bin/env bash
set -euo pipefail

repository="${RELEASE_REPOSITORY:-ArthorMurugan/zed-remote-development}"
bin_dir="${ZRD_BIN_DIR:-$HOME/.local/bin}"
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
extension_dir="$data_home/zrd/extension"
asset="zed-remote-development-linux-x86_64.tar.gz"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
    printf 'This release installer supports Linux x86_64 only.\n' >&2
    exit 1
fi

for command_name in curl tar install; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        printf '%s is required to install zrd.\n' "$command_name" >&2
        exit 1
    fi
done

curl --fail --location --silent --show-error \
    "https://github.com/$repository/releases/latest/download/$asset" \
    --output "$tmp_dir/$asset"
tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"

mkdir -p "$bin_dir" "$(dirname "$extension_dir")"
install -m 755 "$tmp_dir/zrd" "$bin_dir/zrd"
rm -rf "$extension_dir"
mkdir -p "$extension_dir"
cp -R "$tmp_dir/extension/." "$extension_dir/"

printf 'Installed zrd at %s/zrd\n' "$bin_dir"
printf 'Staged extension and WASM at %s\n' "$extension_dir"
printf 'In Zed, run "zed: install dev extension" and select that directory once.\n'
if [[ ":$PATH:" != *":$bin_dir:"* ]]; then
    printf 'Add %s to PATH, then restart Zed.\n' "$bin_dir"
fi

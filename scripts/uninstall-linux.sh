#!/usr/bin/env bash
set -euo pipefail

bin_dir="${ZRD_BIN_DIR:-$HOME/.local/bin}"
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
extension_dir="$data_home/zrd/extension"

rm -f "$bin_dir/zrd"
rm -rf "$extension_dir"
rmdir "$data_home/zrd" 2>/dev/null || true

printf 'Removed %s/zrd and staged extension files.\n' "$bin_dir"
printf 'Remove the dev extension in Zed before running this script.\n'
printf 'Your ZRD machine registrations and session state were left untouched.\n'

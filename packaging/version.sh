#!/usr/bin/env bash
# Prints the workspace version from Cargo.toml, the one number every package carries.
set -euo pipefail
sed -n 's/^version = "\(.*\)"$/\1/p' "$(dirname "$0")/../Cargo.toml" | head -n 1

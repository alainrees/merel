#!/usr/bin/env bash
# Remembers your Apple ID team and the app's bundle identifier for the iPhone app, so they
# survive `xcodegen`. The team id is the ten characters Xcode shows under Signing & Capabilities
# (or in Xcode > Settings > Accounts).
#   apps/ios/scripts/signing.sh <team-id> [bundle-id]
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
team="${1:?usage: signing.sh <team-id> [bundle-id]}"
bundle="${2:-nl.merel.app}"
[[ "$team" =~ ^[A-Z0-9]{10}$ ]] || { echo "A team id is ten letters and digits, like 95SUS87Z39." >&2; exit 1; }
[[ "$bundle" =~ ^[A-Za-z0-9.-]+$ ]] || { echo "A bundle identifier is letters, digits, dots and dashes." >&2; exit 1; }
printf 'DEVELOPMENT_TEAM = %s\nPRODUCT_BUNDLE_IDENTIFIER = %s\n' "$team" "$bundle" > "$here/Config/Local.xcconfig"
echo "Saved in $here/Config/Local.xcconfig. Run xcodegen again, then press Run in Xcode."

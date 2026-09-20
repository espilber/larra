#!/usr/bin/env bash
# Signed, notarized macOS DMG. Loads gitignored .env.signing so you do not
# type the notary password each time.
#
#   ./scripts/release-macos.sh
#   ./scripts/release-macos.sh x86_64-apple-darwin
#
# First codesign may show a Keychain prompt — choose Always Allow.
# beforeBuildCommand fetches only the llama.cpp pin for this target.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env.signing ]]; then
  echo "Missing .env.signing (gitignored). See .env.example." >&2
  exit 1
fi

set -a
# shellcheck disable=SC1091
source .env.signing
set +a

: "${APPLE_SIGNING_IDENTITY:?Set APPLE_SIGNING_IDENTITY in .env.signing}"
: "${APPLE_ID:?Set APPLE_ID in .env.signing}"
: "${APPLE_PASSWORD:?Set APPLE_PASSWORD in .env.signing}"
: "${APPLE_TEAM_ID:?Set APPLE_TEAM_ID in .env.signing}"
: "${TAURI_SIGNING_PRIVATE_KEY:?Set TAURI_SIGNING_PRIVATE_KEY in .env.signing (in-app updater)}"

# xberg/tesseract uses C++ filesystem (macOS 10.15+). Tauri's Intel default is older.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

UPDATER_CONFIG="$(mktemp)"
cleanup() { rm -f "$UPDATER_CONFIG"; }
trap cleanup EXIT
printf '%s\n' '{"bundle":{"createUpdaterArtifacts":true}}' > "$UPDATER_CONFIG"

HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
TARGET="${1:-}"
TRIPLE="${TARGET:-$HOST_TRIPLE}"
if [[ -n "$TARGET" ]]; then
  rustup target add "$TARGET"
  echo "Staging engine pin for ${TARGET}…"
  node scripts/fetch-engine.mjs --triple="$TARGET"
  echo "Building signed DMG for ${TARGET}…"
  pnpm tauri build --target "$TARGET" --bundles app,dmg --config "$UPDATER_CONFIG"
  PREFIX="src-tauri/target/${TARGET}/release/bundle"
else
  echo "Staging engine pin for this Mac…"
  node scripts/fetch-engine.mjs
  echo "Building signed DMG for this Mac…"
  pnpm tauri build --bundles app,dmg --config "$UPDATER_CONFIG"
  PREFIX="src-tauri/target/release/bundle"
fi

echo
echo "Artifacts:"
ls -lh "${PREFIX}/macos" "${PREFIX}/dmg" 2>/dev/null || ls -lh "${PREFIX}"

shopt -s nullglob
dmgs=("${PREFIX}/dmg"/*.dmg "${PREFIX}/macos"/*.dmg)
if [[ ${#dmgs[@]} -eq 0 ]]; then
  echo "No DMG produced under ${PREFIX}." >&2
  exit 1
fi

for dmg in "${dmgs[@]}"; do
  echo
  echo "stapler ${dmg}"
  if ! xcrun stapler validate "$dmg"; then
    echo "No ticket on the DMG (Tauri notarized the .app). Submitting the disk image…"
    xcrun notarytool submit "$dmg" \
      --apple-id "$APPLE_ID" \
      --password "$APPLE_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" \
      --wait
    xcrun stapler staple "$dmg"
    xcrun stapler validate "$dmg"
  fi
  echo "sha256 $(shasum -a 256 "$dmg" | awk '{print $1}')  $(basename "$dmg")"
done

echo
node scripts/latest-json.mjs --bundle-dir "${PREFIX}/macos" --triple "$TRIPLE"
echo
echo "Attach the DMG, the renamed darwin .app.tar.gz from dist/larra-v*, and dist/latest.json"
echo "to the GitHub Release (merge fragments from every platform first:"
echo "  node scripts/latest-json.mjs --combine)."

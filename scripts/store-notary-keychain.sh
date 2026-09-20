#!/usr/bin/env bash
# One-time: copy the notary password from .env.signing into the login
# Keychain. After this you can set APPLE_PASSWORD=@keychain:larra-notary
# in .env.signing and delete the plaintext password from that file.
#
#   ./scripts/store-notary-keychain.sh

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env.signing ]]; then
  echo "Missing .env.signing" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1091
source .env.signing
set +a

: "${APPLE_ID:?}"
: "${APPLE_PASSWORD:?}"
: "${APPLE_TEAM_ID:?}"

if [[ "$APPLE_PASSWORD" == @keychain:* ]]; then
  echo "APPLE_PASSWORD is already a keychain reference. Nothing to store."
  exit 0
fi

security add-generic-password -U -a "$APPLE_ID" -s "larra-notary" -w "$APPLE_PASSWORD"
xcrun notarytool store-credentials "larra-notary" \
  --apple-id "$APPLE_ID" \
  --team-id "$APPLE_TEAM_ID" \
  --password "$APPLE_PASSWORD"

echo
echo "Keychain item larra-notary is saved. In .env.signing you can now use:"
echo "  export APPLE_PASSWORD=@keychain:larra-notary"
echo "and remove the app-specific password from that file."

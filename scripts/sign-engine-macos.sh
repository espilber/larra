#!/usr/bin/env bash
# After fetch-engine stages a macOS llama.cpp tar.gz, sign every Mach-O so
# Apple notarization (which unpacks the archive) will accept the .app.
# No-op without APPLE_SIGNING_IDENTITY or on non-Darwin.

set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  exit 0
fi
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STAGE="$ROOT/src-tauri/resources/engine"
ENTITLEMENTS="$ROOT/scripts/engine.entitlements.plist"

shopt -s nullglob
archives=("$STAGE"/*.tar.gz)
if [[ ${#archives[@]} -ne 1 ]]; then
  echo "sign-engine-macos: expected one staged .tar.gz in ${STAGE}" >&2
  exit 1
fi
ARCHIVE="${archives[0]}"

is_macho() {
  local magic
  magic="$(xxd -p -l 4 "$1" 2>/dev/null || true)"
  case "$magic" in
    cffaedfe | cefaedfe | feedface | feedfacf | cafebabe | cafed00d) return 0 ;;
    *) return 1 ;;
  esac
}

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/larra-engine-sign.XXXXXX")"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

mkdir -p "$WORKDIR/src"
tar -xzf "$ARCHIVE" -C "$WORKDIR/src"

signed=0
while IFS= read -r -d '' file; do
  if is_macho "$file"; then
    codesign --force --options runtime --timestamp \
      --entitlements "$ENTITLEMENTS" \
      --sign "$APPLE_SIGNING_IDENTITY" \
      "$file"
    signed=$((signed + 1))
  fi
done < <(find "$WORKDIR/src" -type f -print0)

if [[ "$signed" -eq 0 ]]; then
  echo "sign-engine-macos: no Mach-O files in ${ARCHIVE}" >&2
  exit 1
fi

export COPYFILE_DISABLE=1
OUT="$WORKDIR/out.tar.gz"
tar -czf "$OUT" -C "$WORKDIR/src" .
mv -f "$OUT" "$ARCHIVE"
echo "signed ${signed} Mach-O files in $(basename "$ARCHIVE")"

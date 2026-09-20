#!/usr/bin/env bash
# Throwaway DMG that mirrors Tauri's bundle_dmg.sh window (background, icon
# positions, hidden-file shove, toolbar off). Not signed. For layout checks.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DMG_DIR="$ROOT/src-tauri/dmg"
BACKGROUND="$DMG_DIR/background.png"
ICON="$ROOT/src-tauri/icons/icon.icns"
STAGING="$(mktemp -d /tmp/larra-dmg-preview.XXXXXX)"
OUT="$DMG_DIR/preview-Larra.dmg"
VOL="Larra"
VOL_DEV=""

WINX=200
WINY=120
WINW=660
WINH=428
APP_X=168
APP_Y=178
APPS_X=492
APPS_Y=178
ICON_SIZE=128
TEXT_SIZE=16

cleanup() {
  hdiutil detach "$VOL_DEV" >/dev/null 2>&1 || true
  if [[ -d /Volumes/$VOL ]]; then
    hdiutil detach "/Volumes/$VOL" >/dev/null 2>&1 || true
  fi
  rm -rf "$STAGING"
}
trap cleanup EXIT

if [[ ! -f "$BACKGROUND" ]]; then
  echo "Missing $BACKGROUND — run: just dmg-background" >&2
  exit 1
fi
if [[ ! -f "$ICON" ]]; then
  echo "Missing $ICON" >&2
  exit 1
fi

if [[ -d /Volumes/$VOL ]]; then
  hdiutil detach "/Volumes/$VOL" >/dev/null 2>&1 || true
fi
rm -f "$OUT" "$DMG_DIR/rw.preview.dmg"

APP="$STAGING/Larra.app/Contents"
mkdir -p "$APP/MacOS" "$APP/Resources"
cp "$ICON" "$APP/Resources/AppIcon.icns"
cat > "$APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>Larra</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIdentifier</key>
  <string>io.larra.desktop.dmg-preview</string>
  <key>CFBundleName</key>
  <string>Larra</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
</dict>
</plist>
PLIST
printf '#!/bin/sh\nexit 0\n' > "$APP/MacOS/Larra"
chmod +x "$APP/MacOS/Larra"

hdiutil create -ov -volname "$VOL" -fs HFS+ -srcfolder "$STAGING" -format UDRW "$DMG_DIR/rw.preview.dmg" >/dev/null
hdiutil resize -size 40m "$DMG_DIR/rw.preview.dmg" >/dev/null
ATTACH="$(hdiutil attach -readwrite -noverify -noautoopen "$DMG_DIR/rw.preview.dmg")"
VOL_DEV="$(echo "$ATTACH" | awk '/^\/dev\// { print $1; exit }')"
MOUNT="$(echo "$ATTACH" | awk '/\/Volumes\// { print $3; exit }')"
if [[ -z "$MOUNT" || ! -d "$MOUNT" ]]; then
  echo "Failed to mount preview image" >&2
  exit 1
fi

mkdir -p "$MOUNT/.background"
cp "$BACKGROUND" "$MOUNT/.background/background.png"
ln -s /Applications "$MOUNT/Applications"
cp "$ICON" "$MOUNT/.VolumeIcon.icns"
xcrun SetFile -c icnC "$MOUNT/.VolumeIcon.icns"
xcrun SetFile -a C "$MOUNT"

# Same clauses Tauri's bundle_dmg.sh injects when --background is set.
osascript <<OSA
tell application "Finder"
  tell disk "$VOL"
    open
    set theXOrigin to $WINX
    set theYOrigin to $WINY
    set theWidth to $WINW
    set theHeight to $WINH
    set theBottomRightX to (theXOrigin + theWidth)
    set theBottomRightY to (theYOrigin + theHeight)
    tell container window
      set current view to icon view
      set toolbar visible to false
      set statusbar visible to false
      set the bounds to {theXOrigin, theYOrigin, theBottomRightX, theBottomRightY}
      set position of every item to {theBottomRightX + 100, 100}
    end tell
    set opts to the icon view options of container window
    tell opts
      set icon size to $ICON_SIZE
      set text size to $TEXT_SIZE
      set arrangement to not arranged
    end tell
    set background picture of opts to file ".background:background.png"
    set position of item "Larra.app" to {$APP_X, $APP_Y}
    set the extension hidden of item "Larra.app" to true
    set position of item "Applications" to {$APPS_X, $APPS_Y}
    close
    open
    delay 1
    tell container window
      set statusbar visible to false
      set the bounds to {theXOrigin, theYOrigin, theBottomRightX - 10, theBottomRightY - 10}
    end tell
  end tell
  delay 1
  tell disk "$VOL"
    tell container window
      set statusbar visible to false
      set the bounds to {$WINX, $WINY, $WINX + $WINW, $WINY + $WINH}
    end tell
  end tell
  delay 2
end tell
OSA

rm -rf "$MOUNT/.fseventsd" || true
sync
hdiutil detach "$VOL_DEV" >/dev/null
VOL_DEV=""
hdiutil convert -ov "$DMG_DIR/rw.preview.dmg" -format UDZO -imagekey zlib-level=9 -o "$OUT" >/dev/null
rm -f "$DMG_DIR/rw.preview.dmg"

echo "Opening $OUT"
open "$OUT"
echo "Hidden files (.VolumeIcon.icns, .background) should sit off to the right."
echo "Finder will not hard-lock the size; this pins the default bounds and hides the toolbar."

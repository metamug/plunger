#!/usr/bin/env bash
# Builds Plunger.app and a disk image from the compiled binary (or two, one per CPU, which are joined
# into one universal app). Run it on a Mac.
#
#   packaging/macos/build-app.sh VERSION OUT_DIR BINARY [SECOND_BINARY]
#
# Writes OUT_DIR/Plunger.app, OUT_DIR/Plunger-VERSION-macos.dmg and its .sha256.
# The app is signed ad hoc (no Apple Developer ID), so a person opening it for the first time has to
# approve it once; see docs/packaging-macos.md for what signing and notarization would add.
set -euo pipefail

VERSION=${1:?version, for example 0.5.3}
OUT=${2:?output directory}
shift 2
[ "$#" -ge 1 ] || { echo "give the compiled binary (and optionally a second one for the other CPU)"; exit 1; }

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
APP="$OUT/Plunger.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$OUT"

if [ "$#" -ge 2 ]; then
  lipo -create -output "$APP/Contents/MacOS/plunger" "$1" "$2"
else
  cp "$1" "$APP/Contents/MacOS/plunger"
fi
chmod +x "$APP/Contents/MacOS/plunger"

# The icon: the 1024 px artwork, scaled to every size an .icns holds.
ICONSET="$(mktemp -d)/Plunger.iconset"
mkdir -p "$ICONSET"
SRC="$ROOT/packaging/icons/app-1024.png"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$SRC" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$SRC" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Plunger.icns"

sed "s/@VERSION@/$VERSION/g" "$HERE/Info.plist.in" > "$APP/Contents/Info.plist"
cp "$ROOT/LICENSE" "$APP/Contents/Resources/LICENSE"

# Apple Silicon will not run unsigned code, so sign ad hoc.
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

# The disk image: the app and a shortcut to /Applications to drag it onto.
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
DMG="$OUT/Plunger-$VERSION-macos.dmg"
rm -f "$DMG"
hdiutil create -volname "Plunger $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
(cd "$OUT" && shasum -a 256 "Plunger-$VERSION-macos.dmg" > "Plunger-$VERSION-macos.dmg.sha256")

echo "built $APP"
file "$APP/Contents/MacOS/plunger"
lipo -info "$APP/Contents/MacOS/plunger" || true
cat "$OUT/Plunger-$VERSION-macos.dmg.sha256"

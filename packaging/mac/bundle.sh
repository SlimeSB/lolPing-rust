#!/bin/sh
# Wraps the release binary and the assets into lolPing.app, signed ad hoc so macOS runs it at all.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
VERSION=${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)}
# Info.plist takes three integers and nothing else, so a pre-release suffix stays out of it.
PLIST_VERSION=${VERSION%%-*}
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
APP="$TARGET/release/lolPing.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$TARGET/release/lolPing" "$APP/Contents/MacOS/lolPing"
cp -R "$ROOT/assets" "$APP/Contents/Resources/assets"
sed "s/__VERSION__/$PLIST_VERSION/g" "$ROOT/packaging/mac/Info.plist" > "$APP/Contents/Info.plist"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
iconset="$tmp/AppIcon.iconset"
mkdir -p "$iconset"
icon() { sips -z "$2" "$2" "$ROOT/packaging/mac/icon.png" --out "$iconset/$1" >/dev/null; }
icon icon_16x16.png 16
icon icon_16x16@2x.png 32
icon icon_32x32.png 32
icon icon_32x32@2x.png 64
icon icon_128x128.png 128
icon icon_128x128@2x.png 256
icon icon_256x256.png 256
icon icon_256x256@2x.png 512
icon icon_512x512.png 512
icon icon_512x512@2x.png 1024
iconutil -c icns "$iconset" -o "$APP/Contents/Resources/AppIcon.icns"

codesign --force --sign - "$APP"
echo "$APP"

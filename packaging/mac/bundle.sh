#!/bin/sh
# Wraps the release binary and the assets into lolPing.app, signed ad hoc so macOS runs it at all.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
VERSION=${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)}
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
APP="$TARGET/release/lolPing.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$TARGET/release/lolPing" "$APP/Contents/MacOS/lolPing"
cp -R "$ROOT/assets" "$APP/Contents/MacOS/assets"
sed "s/__VERSION__/$VERSION/g" "$ROOT/packaging/mac/Info.plist" > "$APP/Contents/Info.plist"
codesign --force --sign - "$APP"
echo "$APP"

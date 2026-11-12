#!/bin/sh
# Make "Dawn FM.app" from a built binary:
#   macos/bundle.sh target/release/dawnfm 2.1.0 out [portable]
# With `portable`, the app keeps its data in a folder beside itself (see portable.txt).
# The app is signed ad hoc (there's no Developer ID), so macOS asks before its first
# launch: right-click it and pick Open, or run `xattr -dr com.apple.quarantine "Dawn FM.app"`.
set -e

bin=$1
version=$2
out=$3
kind=$4
[ -n "$bin" ] && [ -n "$version" ] && [ -n "$out" ] || { echo "usage: $0 <binary> <version> <out dir>"; exit 1; }
here=$(cd "$(dirname "$0")" && pwd)
icon="$here/../assets/icon/icon.png"
# APP_NAME and EXEC rename the bundle and its binary: releases also carry a copy named as
# Dusk FM was ("Dusk FM.app" running "duskfm"), which Dusk FM 2.1.0's updater looks for.
name=${APP_NAME:-Dawn FM}
exec_name=${EXEC:-dawnfm}
app="$out/$name.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/$exec_name"
chmod 755 "$app/Contents/MacOS/$exec_name"
sed -e "s/@VERSION@/$version/g" -e "s/@EXEC@/$exec_name/g" "$here/Info.plist" > "$app/Contents/Info.plist"

# The Dock and Finder icon, at every size iconutil takes, from the 1024 px source.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
set=$tmp/dawnfm.iconset
mkdir "$set"
for spec in 16:16x16 32:16x16@2x 32:32x32 64:32x32@2x 128:128x128 256:128x128@2x 256:256x256 512:256x256@2x 512:512x512 1024:512x512@2x; do
    sips -z "${spec%%:*}" "${spec%%:*}" "$icon" --out "$set/icon_${spec#*:}.png" >/dev/null
done
iconutil -c icns "$set" -o "$app/Contents/Resources/dawnfm.icns"

# Before signing: the signature seals everything in Resources.
[ "$kind" = portable ] && cp "$here/portable.txt" "$app/Contents/Resources/portable.txt"

codesign --force --sign - "$app"
echo "Made $app"

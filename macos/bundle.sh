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
appicons_root="$here/../AppIcons"
iconset_26="$here/../assets/icon/AppIcon_macos26.iconset"
iconset_under26="$here/../assets/icon/AppIcon.iconset"
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

# Detect OS version: macOS 26+ vs under 26
os_major=$(sw_vers -productVersion 2>/dev/null | cut -d. -f1 || echo 0)
darwin_major=$(uname -r 2>/dev/null | cut -d. -f1 || echo 0)

# Build macOS 26+ icon if available so bundle has both
if [ -d "$iconset_26" ]; then
    iconutil -c icns "$iconset_26" -o "$app/Contents/Resources/dawnfm-26.icns"
fi

# The Dock and Finder icon: under 26 uses AppIcons folder in project root; 26+ uses iconset_26
if [ "$os_major" -ge 26 ] 2>/dev/null || [ "$darwin_major" -ge 26 ] 2>/dev/null; then
    if [ -f "$app/Contents/Resources/dawnfm-26.icns" ]; then
        cp "$app/Contents/Resources/dawnfm-26.icns" "$app/Contents/Resources/dawnfm.icns"
    elif [ -d "$iconset_26" ]; then
        iconutil -c icns "$iconset_26" -o "$app/Contents/Resources/dawnfm.icns"
    fi
else
    # Under 26: use AppIcons folder in project root
    if [ -d "$iconset_under26" ]; then
        iconutil -c icns "$iconset_under26" -o "$app/Contents/Resources/dawnfm.icns"
    elif [ -d "$appicons_root/Assets.xcassets/AppIcon.appiconset" ]; then
        tmp_set=$(mktemp -d)
        set=$tmp_set/dawnfm.iconset
        mkdir -p "$set"
        appiconset="$appicons_root/Assets.xcassets/AppIcon.appiconset"
        cp "$appiconset/16.png" "$set/icon_16x16.png"
        cp "$appiconset/32.png" "$set/icon_16x16@2x.png"
        cp "$appiconset/32.png" "$set/icon_32x32.png"
        cp "$appiconset/64.png" "$set/icon_32x32@2x.png"
        cp "$appiconset/128.png" "$set/icon_128x128.png"
        cp "$appiconset/256.png" "$set/icon_128x128@2x.png"
        cp "$appiconset/256.png" "$set/icon_256x256.png"
        cp "$appiconset/512.png" "$set/icon_256x256@2x.png"
        cp "$appiconset/512.png" "$set/icon_512x512.png"
        cp "$appiconset/1024.png" "$set/icon_512x512@2x.png"
        iconutil -c icns "$set" -o "$app/Contents/Resources/dawnfm.icns"
        rm -rf "$tmp_set"
    else
        tmp=$(mktemp -d)
        trap 'rm -rf "$tmp"' EXIT
        set=$tmp/dawnfm.iconset
        mkdir "$set"
        for spec in 16:16x16 32:16x16@2x 32:32x32 64:32x32@2x 128:128x128 256:128x128@2x 256:256x256 512:256x256@2x 512:512x512 1024:512x512@2x; do
            sips -z "${spec%%:*}" "${spec%%:*}" "$icon" --out "$set/icon_${spec#*:}.png" >/dev/null
        done
        iconutil -c icns "$set" -o "$app/Contents/Resources/dawnfm.icns"
    fi
fi

# Before signing: the signature seals everything in Resources.
[ "$kind" = portable ] && cp "$here/portable.txt" "$app/Contents/Resources/portable.txt"

codesign --force --sign - "$app"
echo "Made $app"

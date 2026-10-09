#!/bin/zsh
# Packs dist/0xTerm.app into dist/0xTerm-<version>.dmg: open it, drag 0xTerm to Applications.
set -euo pipefail
cd "${0:A:h}/.."

app=dist/0xTerm.app
[[ -d "$app" ]] || packaging/bundle.sh
version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")
dmg="dist/0xTerm-$version.dmg"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -volname 0xTerm -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg" >/dev/null
echo "built $dmg ($(du -h "$dmg" | cut -f1))"

#!/bin/zsh
# Builds the official icon set from assets/icons into dist/icons:
#   <name>.icns       compiled from <name>.icon by actool, as AppIcon is; bundled
#                     in the app and offered on 0xterm.org/icons/
#   <name>.svg        the flat, logo-shaped artwork, importable in Settings
#   0xterm-icons.zip  all of the above plus the Icon Composer sources
#   packaging/icons.sh
set -euo pipefail
cd "${0:A:h}/.."

out=dist/icons
rm -rf "$out" dist/0xterm-icons
mkdir -p "$out" dist/0xterm-icons
for icon in assets/icons/*.icon; do
  name=${icon:t:r}
  tmp=$(mktemp -d)
  # actool resolves relative paths against its daemon's directory, hence $PWD.
  xcrun actool "$PWD/$icon" --compile "$tmp" --app-icon "$name" \
    --platform macosx --target-device mac --minimum-deployment-target 13.0 \
    --output-partial-info-plist "$tmp/icon.plist" > /dev/null
  cp "$tmp/$name.icns" "$out/"
  cp "assets/icons/$name.svg" "$out/"
  ditto "$icon" "dist/0xterm-icons/$name.icon"
  rm -rf "$tmp"
done
cp "$out"/*.icns "$out"/*.svg dist/0xterm-icons/
rm -f "$out/0xterm-icons.zip"
(cd dist && zip -qr -X icons/0xterm-icons.zip 0xterm-icons)
rm -rf dist/0xterm-icons
echo "$PWD/$out"

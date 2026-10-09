#!/bin/zsh
# Builds dist/0xTerm.app from a release build.
#   packaging/bundle.sh                 ad-hoc signed: runs on this Mac
#   SIGN_ID="Developer ID Application: …" packaging/bundle.sh
#                                       signed with the hardened runtime, ready to notarize
set -euo pipefail
cd "${0:A:h}/.."

version=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="oxterm"))')
build=$(git rev-list --count HEAD 2>/dev/null || echo 1)

cargo build --release -p oxterm

app=dist/0xTerm.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/oxterm "$app/Contents/MacOS/oxterm"
# Icon Composer icon (logo.svg split into bezel and screen layers, full bleed so
# macOS 26+ masks it instead of setting it on a grey plate): Assets.car + AppIcon.icns.
# actool resolves relative paths against its daemon's directory, hence $PWD.
xcrun actool "$PWD/assets/AppIcon.icon" --compile "$PWD/$app/Contents/Resources" --app-icon AppIcon \
  --platform macosx --target-device mac --minimum-deployment-target 13.0 \
  --output-partial-info-plist "$(mktemp -d)/icon.plist" > /dev/null
sed -e "s/__VERSION__/$version/" -e "s/__BUILD__/$build/" packaging/Info.plist > "$app/Contents/Info.plist"

if [[ -n "${SIGN_ID:-}" ]]; then
  codesign --force --options runtime --timestamp --entitlements packaging/entitlements.plist -s "$SIGN_ID" "$app"
else
  codesign --force -s - "$app"
fi
codesign --verify --strict "$app"
echo "built $app ($version, build $build)"

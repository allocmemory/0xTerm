#!/bin/zsh
# Builds a release from a clean, tagged main: signs with the Developer ID,
# notarizes and staples the app and the DMG, writes the .sha256, and adds the
# version to the Sparkle feed in the site checkout.
#   packaging/release.sh
# Needs, all in the login keychain: the "Developer ID Application" certificate,
# the notarytool profile "0xterm" (NOTARY_PROFILE to change it), and Sparkle's
# signing key (account "0xterm"). SITE_DIR is the 0xTermSite checkout
# (default ../0xTermSite).
set -euo pipefail
cd "${0:A:h}/.."

: ${SIGN_ID:=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: .*\)"$/\1/p' | head -1)}
[[ -n "$SIGN_ID" ]] || { echo "release: no Developer ID Application certificate in the keychain" >&2; exit 1; }
export SIGN_ID
profile=${NOTARY_PROFILE:-0xterm}
site=${SITE_DIR:-../0xTermSite}
[[ -f "$site/zola.toml" ]] || { echo "release: $site isn't the 0xTermSite checkout (set SITE_DIR)" >&2; exit 1; }

# Waits for Apple's verdict; notarytool exits 0 even when it rejects a build.
notarize() {
  local result id
  result=$(xcrun notarytool submit "$1" --keychain-profile "$profile" --wait --output-format json)
  id=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$result")
  if [[ $(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["status"])' "$result") != Accepted ]]; then
    xcrun notarytool log "$id" --keychain-profile "$profile" >&2
    echo "release: notarization of $1 failed" >&2
    exit 1
  fi
  echo "notarized $1 ($id)"
}

packaging/bundle.sh
app=dist/0xTerm.app
version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")
dmg="dist/0xTerm-$version.dmg"

# The app first, so it carries its own ticket once it's out of the DMG.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
ditto -c -k --keepParent "$app" "$stage/0xTerm.zip"
notarize "$stage/0xTerm.zip"
xcrun stapler staple -q "$app"

packaging/dmg.sh
codesign --force --timestamp -s "$SIGN_ID" "$dmg"
notarize "$dmg"
xcrun stapler staple -q "$dmg"

spctl --assess --type execute "$app"
spctl --assess --type open --context context:primary-signature "$dmg"
(cd dist && shasum -a 256 "${dmg:t}" > "${dmg:t}.sha256")

sparkle=$(packaging/sparkle.sh)
signature=$("$sparkle/bin/sign_update" --account 0xterm "$dmg")
python3 packaging/appcast.py "$site/static/appcast.xml" "$app/Contents/Info.plist" "$dmg" "$signature"
echo "released $dmg; next: gh release create v$version $dmg $dmg.sha256, then a site PR for static/appcast.xml"

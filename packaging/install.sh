#!/bin/zsh
# Builds 0xTerm from this checkout and installs it to /Applications, replacing the old copy.
#   packaging/install.sh
# A running 0xTerm keeps working; quit and reopen it to use the new build.
set -euo pipefail
cd "${0:A:h}/.."

packaging/bundle.sh
rm -rf /Applications/0xTerm.app
ditto dist/0xTerm.app /Applications/0xTerm.app
echo "installed /Applications/0xTerm.app"
if pgrep -f /Applications/0xTerm.app/Contents/MacOS/oxterm > /dev/null; then
  echo "0xTerm is running: quit it (⌘Q) and reopen it to use the new build."
fi

#!/bin/zsh
# Downloads the pinned Sparkle release once into target/sparkle/<version> and
# prints that folder: Sparkle.framework is embedded by bundle.sh, and bin/
# holds the tools release.sh signs updates with.
#   packaging/sparkle.sh
set -euo pipefail
cd "${0:A:h}/.."

version=2.10.0
sha256=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c

dir=target/sparkle/$version
if [[ ! -d "$dir/Sparkle.framework" ]]; then
  mkdir -p "$dir"
  archive="$dir/Sparkle-$version.tar.xz"
  curl -fsSL -o "$archive" "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-$version.tar.xz"
  echo "$sha256  $archive" | shasum -a 256 -c --quiet
  tar -xJf "$archive" -C "$dir"
  rm "$archive"
fi
echo "$PWD/$dir"

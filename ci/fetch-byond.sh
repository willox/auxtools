#!/bin/sh
# Fetches a BYOND build and unpacks it, for the tests that run worlds:
#
#     ci/fetch-byond.sh VERSION linux|windows DIRECTORY
#
# DIRECTORY/byond is the build afterwards, the directory with bin/ in it.
#
# Try the BYOND build mirror first, then byond.com. BYOND_MIRROR overrides
# the mirror URL. Verify against ci/byond.sha256 before extraction.
set -eu
[ $# -eq 3 ] || { sed -n '2,6p' "$0" >&2; exit 2; }
version=$1 os=$2 dest=$3
case "$os" in
	linux) zip=${version}_byond_linux.zip ;;
	windows) zip=${version}_byond.zip ;;
	*) echo "$0: \"$os\" is neither linux nor windows" >&2; exit 2 ;;
esac
major=${version%%.*}

expected=$(sed -n "s/^\([0-9a-f]\{64\}\)  $zip\$/\1/p" "$(dirname "$0")/byond.sha256")
[ -n "$expected" ] || { echo "ci/byond.sha256 has no hash for $zip" >&2; exit 1; }

if [ -d "$dest/byond/bin" ] && [ "$(cat "$dest/sha256" 2>/dev/null)" = "$expected" ]; then
	echo "$dest has BYOND $version for $os already"
	exit 0
fi

rm -rf "$dest"
mkdir -p "$dest"
for url in "${BYOND_MIRROR:-https://byond-builds.dm-lang.org}/$major/$zip" "https://www.byond.com/download/build/$major/$zip"; do
	echo "fetching $url"
	if curl -fsSL --retry 2 -A "auxtools tests" -o "$dest/$zip" "$url"; then
		actual=$(sha256sum "$dest/$zip" | cut -c 1-64)
		[ "$actual" = "$expected" ] && break
		echo "SHA-256 mismatch for $zip: got $actual, expected $expected" >&2
	fi
	rm -f "$dest/$zip"
done
[ -f "$dest/$zip" ] || { echo "couldn't fetch $zip" >&2; exit 1; }

unzip -q "$dest/$zip" -d "$dest"
rm "$dest/$zip"
[ -d "$dest/byond/bin" ] || { echo "$zip has no byond/bin" >&2; exit 1; }
echo "$expected" >"$dest/sha256"
echo "BYOND $version for $os is in $dest/byond"

#!/bin/sh
# Fetch Microsoft C runtime and Windows SDK headers/import libraries
# for Linux cross-compilation to i686-pc-windows-msvc.
#
#     ci/msvc/fetch-sdk.sh DIRECTORY
#
# Use xwin (https://github.com/Jake-Shadle/xwin) to download and extract
# the libraries into DIRECTORY (about 800 MB). Running this script accepts
# Microsoft's license: https://go.microsoft.com/fwlink/?LinkId=2086102
#
# Pin SDK and CRT versions for reproducible linking. Static CRT linking
# and the selected system imports support Windows 10 and later.
set -eu
[ $# -eq 1 ] || { sed -n '2,8p' "$0" >&2; exit 2; }
dest=$1

XWIN_VERSION=0.10.0
XWIN_SHA256=d870eb4b2f390878af6da1ccd3cf321d22fcb72720984853b4be732ae597fc88
SDK_VERSION=10.0.26100
CRT_VERSION=14.44.17.14

stamp="xwin $XWIN_VERSION, SDK $SDK_VERSION, CRT $CRT_VERSION"
if [ "$(cat "$dest/version" 2>/dev/null)" = "$stamp" ]; then
	echo "$dest has $stamp already"
	exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fsSL --retry 3 -o "$work/xwin.tar.gz" \
	"https://github.com/Jake-Shadle/xwin/releases/download/$XWIN_VERSION/xwin-$XWIN_VERSION-x86_64-unknown-linux-musl.tar.gz"
echo "$XWIN_SHA256  $work/xwin.tar.gz" | sha256sum -c -
tar -xzf "$work/xwin.tar.gz" -C "$work" --strip-components=1

rm -rf "$dest"
"$work/xwin" --accept-license --arch x86 --sdk-version "$SDK_VERSION" --crt-version "$CRT_VERSION" \
	--cache-dir "$work/cache" splat --output "$dest"
echo "$stamp" >"$dest/version"

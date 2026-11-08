#!/bin/sh
# setup-spacemandmm.sh [DIR]
#
# dmasm's proc compiler builds against SpacemanDMM's `dreammaker` crate with
# parser fixes that aren't upstream yet. dmasm keeps them as a `git am` series,
# patches/spacemandmm.patch, on SpaceManiac/SpacemanDMM a388985. Cargo.toml
# [patch]es `dreammaker` with a checkout at ../SpacemanDMM.
#
# This clones SpacemanDMM into DIR (default: ../SpacemanDMM next to this
# repository) and applies the series from the dmasm commit Cargo.lock pins.
# An existing DIR is left alone, so a checkout you're working in is safe.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
dir=${1:-$here/../SpacemanDMM}
base=a388985

if [ -e "$dir" ]; then
	echo "$dir already exists; leaving it as it is"
	exit 0
fi

# source = "git+https://github.com/OWNER/dmasm?branch=NAME#REV"
source=$(grep -A2 '^name = "dmasm"$' "$here/Cargo.lock" | sed -n 's/^source = "git+\(.*\)"$/\1/p')
if [ -z "$source" ]; then
	echo "no git source for dmasm in Cargo.lock" >&2
	exit 1
fi
url=${source%%[?#]*}
rev=${source##*#}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
git clone --quiet --filter=blob:none --no-checkout "$url" "$tmp/dmasm"
git -C "$tmp/dmasm" show "$rev:patches/spacemandmm.patch" > "$tmp/spacemandmm.patch"

git -c core.autocrlf=false clone --quiet https://github.com/SpaceManiac/SpacemanDMM "$dir"
git -C "$dir" checkout --quiet -b dmasm-patches "$base"
git -C "$dir" -c user.name=dmasm -c user.email=dmasm@localhost am --quiet "$tmp/spacemandmm.patch"
echo "SpacemanDMM $base with dmasm ${rev%"${rev#???????}"}'s parser patches applied in $dir"

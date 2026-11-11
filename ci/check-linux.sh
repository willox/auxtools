#!/bin/sh
# Check 32-bit Linux runtime requirements against Ubuntu 22.04:
# glibc <= 2.35, libstdc++ <= 12, and an explicit shared-library allowlist.
#
#     ci/check-linux.sh FILE...
set -eu
GLIBC=2.35
GLIBCXX=3.4.30
LIBRARIES="libc.so.6 libm.so.6 libgcc_s.so.1 libstdc++.so.6 ld-linux.so.2 libdl.so.2 libpthread.so.0 librt.so.1"

# The newest version of a library's symbols ($2: GLIBC, GLIBCXX) $1 uses
newest() {
	objdump -T "$1" | grep -o "${2}_[0-9][0-9.]*" | cut -d _ -f 2 | sort -uV | tail -n 1
}

later() {
	[ -n "$1" ] && [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1)" = "$1" ]
}

status=0
for file; do
	readelf -h "$file" | grep -q 'Machine: *Intel 80386' || { echo "$file isn't a 32-bit x86 program"; status=1; }
	for library in $(readelf -d "$file" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
		case " $LIBRARIES " in
			*" $library "*) ;;
			*) echo "$file imports unexpected library: $library"; status=1 ;;
		esac
	done
	glibc=$(newest "$file" GLIBC)
	glibcxx=$(newest "$file" GLIBCXX)
	echo "$file: glibc $glibc${glibcxx:+, libstdc++ (GLIBCXX) $glibcxx}"
	if later "$glibc" "$GLIBC"; then echo "$file needs glibc $glibc, newer than $GLIBC"; status=1; fi
	if later "$glibcxx" "$GLIBCXX"; then echo "$file needs GLIBCXX $glibcxx, newer than $GLIBCXX"; status=1; fi
done
exit $status

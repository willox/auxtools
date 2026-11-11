#!/bin/sh
# Extract Linux debug information into adjacent FILE.debug files.
# Windows builds already emit separate PDBs.
#
#     ci/split-debug.sh FILE...
set -eu
for file; do
	objcopy --only-keep-debug "$file" "$file.debug"
	chmod -x "$file.debug"
	objcopy --strip-debug --add-gnu-debuglink="$file.debug" "$file"
done

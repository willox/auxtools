#!/bin/bash
# Run BYOND integration tests using prebuilt binaries:
#
#     ci/run-tests.sh PROGRAMS BYOND [TEST...]
#
# PROGRAMS contains test binaries, launcher and debug library. BYOND is
# the installation directory containing bin/. Run from the repository root.
#
# Set WINE=wine to run Windows binaries on Linux.
#
# With no TEST arguments, also run PROGRAMS/unit tests if that directory exists.
set -u
[ $# -ge 2 ] || { sed -n '2,11p' "$0" >&2; exit 2; }
programs=$(cd "$1" && pwd) || exit 2
byond=$(cd "$2" && pwd) || exit 2
shift 2
tests=${*:-test_runner debug_test hot_reload_test launcher_test dap_test}

exe=
case "$(uname -s)" in
	MINGW* | MSYS* | CYGWIN*)
		exe=.exe
		BYOND_PATH=$(cygpath -w "$byond")
		;;
	*)
		if [ -n "${WINE:-}" ]; then
			exe=.exe
			# Wine has the whole filesystem as drive Z
			BYOND_PATH="Z:$(printf '%s' "$byond" | tr / '\\')"
			export WINEDEBUG="${WINEDEBUG:--all}"
		else
			BYOND_PATH=$byond
			export LD_LIBRARY_PATH="$byond/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
		fi
		;;
esac
export BYOND_PATH

failed=
run() {
	name=$1
	shift
	echo "::group::$name"
	start=$(date +%s)
	${WINE:+"$WINE"} "$@"
	status=$?
	echo "::endgroup::"
	if [ $status -eq 0 ]; then
		echo "passed: $name ($(($(date +%s) - start)) s)"
	else
		echo "FAILED: $name (exit status $status, $(($(date +%s) - start)) s)"
		failed="$failed $name"
	fi
}

for test in $tests; do
	run "$test" "$programs/$test$exe"
done

# cargo runs a package's unit tests from the package's directory
if [ -d "$programs/unit" ] && [ $# -eq 0 ]; then
	root=$PWD
	for test in $(cd "$programs/unit" && find . -type f | sort); do
		cd "$(dirname "$test")" || exit 2
		run "unit tests: ${test#./}" "$programs/unit/$test"
		cd "$root" || exit 2
	done
fi

if [ -n "$failed" ]; then
	echo "failed:$failed"
	exit 1
fi
echo "all passed"

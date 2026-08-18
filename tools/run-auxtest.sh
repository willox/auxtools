#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-all}"

version_from_minor() {
	case "$1" in
		1602|1647)
			printf '515.%s' "$1"
			;;
		1648|1649|1650|1651|1652|1653|1654|1655|1656|1666|1667|1681)
			printf '516.%s' "$1"
			;;
		5??.*)
			printf '%s' "$1"
			;;
		*)
			printf 'Unknown BYOND version or minor build: %s\n' "$1" >&2
			exit 2
			;;
	esac
}

linux_byond_path() {
	local version="$1"
	printf '%s/byond/linux/%s_byond_linux/byond' "$ROOT" "$version"
}

windows_byond_path() {
	local version="$1"
	printf '%s/byond/windows/%s_byond/byond' "$ROOT" "$version"
}

run_linux() {
	local label="$1"
	local byond_path="$2"

	if [[ ! -x "$byond_path/bin/DreamMaker" || ! -x "$byond_path/bin/DreamDaemon" ]]; then
		printf 'Missing Linux BYOND executables for %s at %s\n' "$label" "$byond_path" >&2
		exit 1
	fi

	printf '\n== Linux %s ==\n' "$label"
	(
		cd "$ROOT"
		BYOND_PATH="$byond_path" cargo run -p test_runner --target i686-unknown-linux-gnu
	)
}

windows_vs_install() {
	powershell.exe -NoProfile -Command \
		"\$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'; if (!(Test-Path \$vswhere)) { exit 1 }; \$install = & \$vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath; if (!\$install) { exit 1 }; \$install" \
		| tr -d '\r'
}

run_windows() {
	local label="$1"
	local byond_path="$2"
	local byond_path_win
	local root_win
	local vs_install

	if [[ ! -f "$byond_path/bin/dm.exe" || ! -f "$byond_path/bin/dreamdaemon.exe" ]]; then
		printf 'Missing Windows BYOND executables for %s at %s\n' "$label" "$byond_path" >&2
		exit 1
	fi

	byond_path_win="$(wslpath -w "$byond_path")"
	root_win="$(wslpath -w "$ROOT")"
	vs_install="$(windows_vs_install)"

	if [[ -z "$vs_install" ]]; then
		printf 'Could not find Visual Studio with x86 C++ tools\n' >&2
		exit 1
	fi

	printf '\n== Windows %s ==\n' "$label"
	powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \
		"\$ErrorActionPreference = 'Stop'; "\
"\$repo = '$root_win'; "\
"\$byond = '$byond_path_win'; "\
"\$vsInstall = '$vs_install'; "\
"\$work = Join-Path \$env:TEMP 'auxtools-wsl-$label'; "\
"\$devShell = Join-Path \$vsInstall 'Common7\Tools\Launch-VsDevShell.ps1'; "\
"if (!(Test-Path \$devShell)) { throw \"Could not find \$devShell\" }; "\
"& \$devShell -Arch x86 -HostArch amd64 | Out-Null; "\
"if (!(Test-Path \$work)) { New-Item -ItemType Directory -Path \$work | Out-Null }; "\
"Get-ChildItem -LiteralPath \$work -Force | Where-Object { \$_.Name -ne 'target' } | Remove-Item -Recurse -Force; "\
"Get-ChildItem -LiteralPath \$repo -Force | Where-Object { \$_.Name -notin @('target', '.git', 'byond') } | Copy-Item -Destination \$work -Recurse -Force; "\
"Set-Location \$work; "\
"\$env:BYOND_PATH = \$byond; "\
"\$env:CARGO_INCREMENTAL = '0'; "\
"cargo run -p test_runner --target i686-pc-windows-msvc; "\
"\$status = \$LASTEXITCODE; "\
"exit \$status"
}

run_version() {
	local platform="$1"
	local version
	version="$(version_from_minor "$2")"

	case "$platform" in
		linux)
			run_linux "$version" "$(linux_byond_path "$version")"
			;;
		windows)
			run_windows "$version" "$(windows_byond_path "$version")"
			;;
		*)
			printf 'Unknown platform: %s\n' "$platform" >&2
			exit 2
			;;
	esac
}

case "$TARGET" in
	linux-515)
		run_version linux 1602
		;;
	linux-516)
		run_version linux 1681
		;;
	windows-515)
		run_version windows 1602
		;;
	windows-516)
		run_version windows 1681
		;;
	linux-5??.*|linux-????)
		run_version linux "${TARGET#linux-}"
		;;
	windows-5??.*|windows-????)
		run_version windows "${TARGET#windows-}"
		;;
	5??.*|????)
		run_version windows "$TARGET"
		run_version linux "$TARGET"
		;;
	all)
		run_version windows 1602
		run_version windows 1681
		run_version linux 1602
		run_version linux 1681
		;;
	*)
		printf 'Usage: %s [all|linux-515|linux-516|windows-515|windows-516|linux-<version>|windows-<version>|<version>]\n' "$0" >&2
		printf 'Examples: %s linux-515.1647; %s windows-1647; %s 1602\n' "$0" "$0" "$0" >&2
		exit 2
		;;
esac

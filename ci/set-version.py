#!/usr/bin/env python3
"""Read or set the version shared by the workspace and VS Code extension.

    ci/set-version.py            print the current version
    ci/set-version.py VERSION    set the version, e.g. 3.0.0 or 3.0.1-dev

Update Cargo.toml, inherited crate versions in Cargo.lock and the extension
manifest.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
PACKAGE_JSON = ROOT / "editors" / "vscode" / "package.json"

VERSION = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?$")
WORKSPACE_PACKAGE = re.compile(r"^\[workspace\.package\]$(.*?)(?=^\[|\Z)", re.M | re.S)
VERSION_LINE = re.compile(r'^(version\s*=\s*")([^"]+)(")', re.M)


def replace_once(pattern, replacement, text, what):
	text, count = pattern.subn(replacement, text, count=1)
	if count != 1:
		sys.exit(f"couldn't find {what}")
	return text


def workspace_version(manifest):
	section = WORKSPACE_PACKAGE.search(manifest)
	version = section and VERSION_LINE.search(section.group(1))
	if not version:
		sys.exit("Cargo.toml has no version under [workspace.package]")
	return version.group(2)


def workspace_crates(manifest):
	"""Return crate names that inherit the workspace version."""
	members = re.search(r"^members\s*=\s*\[(.*?)\]", manifest, re.M | re.S)
	if not members:
		sys.exit("Cargo.toml has no workspace members")
	names = []
	for member in re.findall(r'"([^"]+)"', members.group(1)):
		crate = (ROOT / member / "Cargo.toml").read_text()
		if re.search(r"^version\.workspace\s*=\s*true", crate, re.M):
			names.append(re.search(r'^name\s*=\s*"([^"]+)"', crate, re.M).group(1))
	return names


def set_version(version):
	manifest = CARGO_TOML.read_text()
	section = WORKSPACE_PACKAGE.search(manifest)
	body = replace_once(VERSION_LINE, rf"\g<1>{version}\g<3>", section.group(1), "the workspace version")
	CARGO_TOML.write_text(manifest[: section.start(1)] + body + manifest[section.end(1) :])

	# Workspace packages have no source field in Cargo.lock.
	lock = CARGO_LOCK.read_text()
	for name in workspace_crates(manifest):
		entry = re.compile(rf'(\[\[package\]\]\nname = "{re.escape(name)}"\nversion = ")[^"]+(")(?!\nsource)')
		lock = replace_once(entry, rf"\g<1>{version}\g<2>", lock, f"{name} in Cargo.lock")
	CARGO_LOCK.write_text(lock)

	# Preserve package.json's formatting.
	package = PACKAGE_JSON.read_text()
	package = replace_once(re.compile(r'^(\t"version": ")[^"]+(")', re.M), rf"\g<1>{version}\g<2>", package, "the version in package.json")
	if json.loads(package)["version"] != version:
		sys.exit("package.json version update failed")
	PACKAGE_JSON.write_text(package)


def main():
	if len(sys.argv) == 1:
		print(workspace_version(CARGO_TOML.read_text()))
	elif len(sys.argv) == 2 and VERSION.match(sys.argv[1]):
		set_version(sys.argv[1])
	else:
		sys.exit(__doc__)


if __name__ == "__main__":
	main()

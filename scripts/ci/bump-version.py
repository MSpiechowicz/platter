#!/usr/bin/env python3
"""Raises Platter's version in every place that carries it and prints the new version.

    scripts/ci/bump-version.py patch|minor|major [ROOT]

The places: the workspace version in Cargo.toml, the workspace's own packages (platter-*) in Cargo.lock,
and apps/desktop/package.json. Nothing else is touched, and a file whose version line is not found exactly
where it is expected stops the script before anything is written.
"""

import json
import pathlib
import re
import sys

VERSION = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")


def next_version(current: str, bump: str) -> str:
    match = VERSION.match(current)
    if not match:
        sys.exit(f"The version {current!r} is not MAJOR.MINOR.PATCH.")
    major, minor, patch = (int(part) for part in match.groups())
    if bump == "major":
        return f"{major + 1}.0.0"
    if bump == "minor":
        return f"{major}.{minor + 1}.0"
    if bump == "patch":
        return f"{major}.{minor}.{patch + 1}"
    sys.exit(f"Unknown bump {bump!r}: use patch, minor or major.")


def main() -> None:
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    bump = sys.argv[1]
    root = pathlib.Path(sys.argv[2] if len(sys.argv) == 3 else ".")

    cargo_toml = root / "Cargo.toml"
    cargo_lock = root / "Cargo.lock"
    package_json = root / "apps/desktop/package.json"

    manifest = cargo_toml.read_text()
    # The first `version = "..."` after [workspace.package], before the next table.
    section = re.search(r"^\[workspace\.package\]\n(?:(?!^\[).*\n)*?version = \"([^\"]+)\"\n", manifest, re.M)
    if not section:
        sys.exit("No version in [workspace.package] of Cargo.toml.")
    current = section.group(1)
    new = next_version(current, bump)

    start, end = section.span(1)
    manifest = manifest[:start] + new + manifest[end:]

    lock = cargo_lock.read_text()
    lock, changed = re.subn(
        r'^(name = "platter-[a-z-]+"\nversion = )"' + re.escape(current) + '"$',
        lambda found: f'{found.group(1)}"{new}"',
        lock,
        flags=re.M,
    )
    listed = re.search(r"^members = \[([^\]]*)\]", manifest, re.M)
    if not listed:
        sys.exit("No members list in Cargo.toml.")
    members = len(re.findall(r'"[^"]+"', listed.group(1)))
    if changed != members:
        sys.exit(f"Cargo.lock: expected {members} workspace packages at {current}, found {changed}.")

    package = json.loads(package_json.read_text())
    if package.get("version") != current:
        sys.exit(f"apps/desktop/package.json has version {package.get('version')!r}, not {current!r}.")
    package_text = package_json.read_text()
    package_text, replaced = re.subn(
        r'^(  "version": )"' + re.escape(current) + '",$', lambda found: f'{found.group(1)}"{new}",', package_text, count=1, flags=re.M
    )
    if replaced != 1:
        sys.exit("apps/desktop/package.json: the version line is not where it is expected.")

    cargo_toml.write_text(manifest)
    cargo_lock.write_text(lock)
    package_json.write_text(package_text)
    print(new)


if __name__ == "__main__":
    main()

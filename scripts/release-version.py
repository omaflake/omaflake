#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# One version everywhere: the workspace's crates (and the versions they name each other with), the guest agent,
# the app's Info.plist. CI runs it on every push; a release runs it with its tag, which must name that version and have
# its section in docs/changelog.md – the section's text is printed to --notes=<file> as the release's notes.
#   scripts/release-version.py [--tag=v<version>] [--notes=<file>]
# Prints the version. Exit status 1 with what differs.
import plistlib
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def option(name):
    return next((a.split("=", 1)[1] for a in sys.argv[1:] if a.startswith(f"--{name}=")), None)


def changelog_section(version):
    """The text under `## [<version>]` up to the next release's heading; None without that heading"""
    lines = (ROOT / "docs/changelog.md").read_text(encoding="utf-8").splitlines()
    start = next((i for i, line in enumerate(lines) if line.startswith(f"## [{version}]")), None)
    if start is None:
        return None
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("## [")), len(lines))
    return "\n".join(lines[start + 1:end]).strip()


def main():
    problems = []
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    version = workspace["package"]["version"]
    for name, dep in workspace["dependencies"].items():
        if isinstance(dep, dict) and "path" in dep and dep.get("version") != version:
            problems.append(f"Cargo.toml: {name} is named with {dep.get('version')}, the workspace is {version}")
    agent = tomllib.loads((ROOT / "nix/agent/Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    if agent != version:
        problems.append(f"nix/agent/Cargo.toml: the guest agent is {agent}, the workspace {version}")
    with open(ROOT / "App/Info.plist", "rb") as f:
        app = plistlib.load(f).get("CFBundleShortVersionString")
    if app != version:
        problems.append(f"App/Info.plist: the app is {app}, the workspace {version}")
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        problems.append(f"Cargo.toml: {version} is not a release version (x.y.z)")

    tag = option("tag")
    if tag is not None:
        if tag != f"v{version}":
            problems.append(f"the tag is {tag}, the code is {version}")
        notes = changelog_section(version)
        if not notes:
            problems.append(f"docs/changelog.md has no section [{version}] (or it is empty)")
        elif (out := option("notes")) is not None:
            Path(out).write_text(notes + "\n", encoding="utf-8")

    for p in problems:
        print(f"::error::{p}" if tag is not None else p, file=sys.stderr)
    if problems:
        sys.exit(1)
    print(version)


if __name__ == "__main__":
    main()

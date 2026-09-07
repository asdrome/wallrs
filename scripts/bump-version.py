#!/usr/bin/env python3
"""Semantic version bumper for wallrs.

Synchronizes version across Cargo.toml, extra/rpm/wallrs.spec, and extra/arch/PKGBUILD.
"""

from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def bump_semver(current: str, bump_type: str) -> str:
    m = re.match(r"^(\d+)\.(\d+)\.(\d+)(.*)$", current)
    if not m:
        raise ValueError(f"Current version '{current}' is not valid semver")
    major, minor, patch = int(m.group(1)), int(m.group(2)), int(m.group(3))
    if bump_type == "major":
        return f"{major + 1}.0.0"
    elif bump_type == "minor":
        return f"{major}.{minor + 1}.0"
    elif bump_type == "patch":
        return f"{major}.{minor}.{patch + 1}"
    elif re.match(r"^\d+\.\d+\.\d+", bump_type):
        return bump_type
    else:
        raise ValueError(
            f"Invalid bump type or version: {bump_type}. Use 'major', 'minor', 'patch', or 'X.Y.Z'"
        )


def main():
    if len(sys.argv) < 2:
        print("Usage: bump-version.py <patch|minor|major|X.Y.Z>", file=sys.stderr)
        sys.exit(1)

    bump_arg = sys.argv[1].strip()

    cargo_toml = ROOT / "Cargo.toml"
    content = cargo_toml.read_text()
    m = re.search(
        r'\[workspace\.package\][^\[]*version\s*=\s*"([^"]+)"',
        content,
        re.MULTILINE | re.DOTALL,
    )
    if not m:
        raise ValueError(
            "Could not find [workspace.package] version in root Cargo.toml"
        )
    current_ver = m.group(1)
    new_ver = bump_semver(current_ver, bump_arg)

    print(f"Bumping version: {current_ver} -> {new_ver}")

    # 1. Update root Cargo.toml
    new_content = re.sub(
        r'(\[workspace\.package\][^\[]*version\s*=\s*")[^"]+(")',
        rf"\g<1>{new_ver}\g<2>",
        content,
        flags=re.MULTILINE | re.DOTALL,
    )
    cargo_toml.write_text(new_content)

    # 2. Update extra/rpm/wallrs.spec
    spec_path = ROOT / "extra" / "rpm" / "wallrs.spec"
    if spec_path.exists():
        spec_content = spec_path.read_text()
        spec_content = re.sub(
            r"^Version:\s+.*$",
            f"Version:        {new_ver}",
            spec_content,
            flags=re.MULTILINE,
        )
        spec_path.write_text(spec_content)

    # 3. Update extra/arch/PKGBUILD
    pkgbuild_path = ROOT / "extra" / "arch" / "PKGBUILD"
    if pkgbuild_path.exists():
        pkg_content = pkgbuild_path.read_text()
        pkg_content = re.sub(
            r"^pkgver=.*$", f"pkgver={new_ver}", pkg_content, flags=re.MULTILINE
        )
        pkgbuild_path.write_text(pkg_content)

    # 4. Refresh Cargo.lock
    subprocess.run(["cargo", "check", "--workspace"], cwd=ROOT, check=True)
    print(
        f"Successfully updated Cargo.toml, Cargo.lock, wallrs.spec, and PKGBUILD to v{new_ver}"
    )


if __name__ == "__main__":
    main()


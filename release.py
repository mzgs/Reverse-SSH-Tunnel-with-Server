#!/usr/bin/env python3
"""Bump the patch version and push a release tag. Requires Python 3.11+ and Git."""

import argparse
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tomllib


def git(*args):
    # Capture Git errors: a remote URL in stderr can contain credentials.
    return subprocess.run(
        ["git", *args], check=True, text=True, capture_output=True
    ).stdout.strip()


def bump(root):
    manifest = (root / "Cargo.toml").read_text()
    lock = (root / "Cargo.lock").read_text()
    package = tomllib.loads(manifest)["package"]
    old = package["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", old):
        raise ValueError("Expected a stable major.minor.patch version in Cargo.toml.")
    major, minor, patch = map(int, old.split("."))
    new = f"{major}.{minor}.{patch + 1}"
    locked = [p for p in tomllib.loads(lock)["package"] if p["name"] == package["name"]]
    if len(locked) != 1 or locked[0]["version"] != old:
        raise ValueError("Cargo.toml and Cargo.lock package versions must match.")
    updates = {}
    for filename, content, prefix in (
        ("Cargo.toml", manifest, r'^\[package\]\n(?:(?!^\[).)*?^version = "'),
        ("Cargo.lock", lock, rf'^name = "{re.escape(package["name"])}"\nversion = "'),
    ):
        changed, count = re.subn(
            rf'({prefix}){re.escape(old)}(")',
            lambda m: m[1] + new + m[2], content, flags=re.M | re.S,
        )
        if count != 1:
            raise ValueError(f"Cannot locate exactly one package version in {filename}.")
        updates[filename] = changed
    return old, new, updates


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true", help="Check and preview without committing or pushing")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    os.chdir(root)
    if git("status", "--porcelain"):
        raise ValueError("Commit or stash all changes before releasing (including the release workflow).")
    git("ls-files", "--error-unmatch", ".github/workflows/release.yml")
    branch = git("symbolic-ref", "--quiet", "--short", "HEAD")
    old, new, updates = bump(root)
    tag = f"v{new}"
    if git("tag", "--list", tag) or git("ls-remote", "--tags", "origin", f"refs/tags/{tag}"):
        raise ValueError(f"Tag {tag} already exists; no files changed.")
    git("fetch", "--quiet", "--no-tags", "origin", branch)
    git("merge-base", "--is-ancestor", "FETCH_HEAD", "HEAD")
    print(f"{old} -> {new}: commit, tag {tag}, push {branch} to origin.", flush=True)
    if args.dry_run:
        return
    for filename, content in updates.items():
        (root / filename).write_text(content)
    git("add", "--", "Cargo.toml", "Cargo.lock")
    git("commit", "-m", f"chore: release {tag}")
    git("tag", "-a", tag, "-m", f"Release {tag}")
    push = ("push", "--atomic", "origin", f"HEAD:refs/heads/{branch}", f"refs/tags/{tag}")
    try:
        git(*push)
    except subprocess.CalledProcessError:
        print(f"Release commit and {tag} are saved locally. After fixing the push failure, retry:\n"
              f"{shlex.join(('git', *push))}", file=sys.stderr)
        raise
    print(f"Pushed {tag}. GitHub Actions will build all four platforms and publish the release.")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(f"Git {error.cmd[1]} failed (exit {error.returncode}); check authentication, branch state, and permissions.")
    except (OSError, ValueError, KeyError) as error:
        sys.exit(str(error))

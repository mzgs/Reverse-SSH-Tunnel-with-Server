"""Run with python3 -m unittest test_release. Never contacts GitHub."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest

from release import bump


class ReleaseTest(unittest.TestCase):
    def test_release_against_local_remote(self):
        source = Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "work"
            root.mkdir()
            remote = Path(directory) / "origin.git"
            env = {**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                   "GIT_TERMINAL_PROMPT": "0"}

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, env=env, text=True, stderr=subprocess.PIPE).strip()

            def release(*args, success=True):
                result = subprocess.run([sys.executable, "release.py", *args], cwd=root, env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
                return result.stdout + result.stderr

            for filename in ("Cargo.toml", "Cargo.lock", "release.py"):
                shutil.copy2(source / filename, root / filename)
            workflow = root / ".github/workflows/release.yml"
            workflow.parent.mkdir(parents=True)
            shutil.copy2(source / ".github/workflows/release.yml", workflow)
            git("init", "-b", "main")
            git("config", "user.name", "Release test")
            git("config", "user.email", "test@example.invalid")
            git("config", "commit.gpgsign", "false")
            git("config", "tag.gpgsign", "false")
            git("init", "--bare", str(remote))
            git("remote", "add", "origin", str(remote))
            git("add", ".")
            git("commit", "-m", "Initial")
            git("push", "-u", "origin", "main")
            old, new, updates = bump(root)
            old_lock = tomllib.loads((root / "Cargo.lock").read_text())["package"]
            new_lock = tomllib.loads(updates["Cargo.lock"])["package"]
            package_name = tomllib.loads(updates["Cargo.toml"])["package"]["name"]
            self.assertEqual([p for p in old_lock if p["name"] != package_name],
                             [p for p in new_lock if p["name"] != package_name])
            self.assertEqual(tomllib.loads(updates["Cargo.toml"])["package"]["version"], new)
            self.assertIn(f"{old} -> {new}", release("--dry-run"))
            self.assertEqual(git("status", "--porcelain"), "")
            self.assertEqual(git("tag", "--list"), "")
            (root / "uncommitted.txt").write_text("keep me")
            self.assertIn("Commit or stash", release(success=False))
            (root / "uncommitted.txt").unlink()
            git("tag", f"v{new}")
            git("push", "origin", f"refs/tags/v{new}")
            git("tag", "-d", f"v{new}")
            self.assertIn("already exists", release(success=False))
            git("push", "origin", f":refs/tags/v{new}")
            self.assertIn("GitHub Actions", release())
            self.assertEqual(git("status", "--porcelain"), "")
            head = git("rev-parse", "HEAD")
            self.assertEqual(git("--git-dir", str(remote), "rev-parse", "refs/heads/main"), head)
            self.assertEqual(git("--git-dir", str(remote), "rev-parse", f"v{new}^{{}}"), head)
            self.assertEqual(tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"], new)
            _, next_version, _ = bump(root)
            hook = remote / "hooks/pre-receive"
            hook.write_text("#!/bin/sh\necho 'secret-token-must-not-leak' >&2\nexit 1\n")
            hook.chmod(0o755)
            failure = release(success=False)
            self.assertIn("git push --atomic", failure)
            self.assertNotIn("secret-token-must-not-leak", failure)
            self.assertEqual(git("--git-dir", str(remote), "rev-parse", "refs/heads/main"), head)
            self.assertEqual(git("--git-dir", str(remote), "tag", "--list", f"v{next_version}"), "")
            hook.unlink()
            git("push", "--atomic", "origin", "HEAD:refs/heads/main", f"refs/tags/v{next_version}")
            git("tag", "-d", f"v{next_version}")
            git("push", "origin", f":refs/tags/v{next_version}")
            git("reset", "--hard", head)
            self.assertIn("Git merge-base failed", release(success=False))


if __name__ == "__main__":
    unittest.main()

"""Run with python3 -m unittest test_release. Never contacts GitHub."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import tomllib
import unittest

from release import bump


class ReleaseTest(unittest.TestCase):
    def test_macos_self_signing(self):
        source = Path(__file__).resolve().parent
        workflow = (source / ".github/workflows/release.yml").read_text()
        step = workflow.split("      - name: Self-sign and package macOS app\n", 1)[1]
        script = textwrap.dedent(step.split("        run: |\n", 1)[1].split("      - name:", 1)[0])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copy2(source / "build.sh", root / "build.sh")
            (root / "resources").mkdir()
            (root / "resources/app-icon.icns").write_bytes(b"icon")
            pin = root / "resources/signing-certificate.sha1"
            shutil.copy2(source / "resources/signing-certificate.sha1", pin)
            identity = pin.read_text().strip()
            for host in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
                binary = root / f"target/{host}/release/reverse-ssh-tunnel-with-server"
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"binary")
            fake_bin = root / "bin"
            fake_bin.mkdir()
            tool = fake_bin / "tool"
            tool.write_text(f"#!{sys.executable}\n" + textwrap.dedent('''\
                import json, os, sys
                from pathlib import Path
                name, args = Path(sys.argv[0]).name, sys.argv[1:]
                with open(os.environ["COMMAND_LOG"], "a") as log:
                    log.write(json.dumps([name, *args]) + "\\n")
                if os.environ.get("FAIL_TOOL") == name:
                    sys.exit(1)
                if name == "codesign" and "--verify" in args and os.environ.get("FAIL_VERIFY"):
                    sys.exit(1)
                if name == "codesign" and "--sign" in args:
                    signer = args[args.index("--sign") + 1]
                    if signer != "-" and signer != os.environ["IMPORTED_IDENTITY"]:
                        sys.exit(1)
                if name == "uname":
                    print("Darwin")
                elif name == "rustup" and "rustc" in args:
                    print("host: " + os.environ["TEST_HOST"])
                elif name == "rustup" and "pkgid" in args:
                    print("app@0.1.5")
                elif name == "openssl":
                    print("temporary-keychain-password")
                elif name == "ditto":
                    Path(args[-1]).write_text("archive")
                '''))
            tool.chmod(0o755)
            for name in ("uname", "rustup", "plutil", "codesign", "security", "openssl", "ditto", "xcrun", "spctl"):
                (fake_bin / name).symlink_to(tool)
            credentials = {
                "MACOS_CERTIFICATE_BASE64": "Y2VydA==",
                "MACOS_CERTIFICATE_PASSWORD": "fake-certificate-password",
            }
            env = {**os.environ, **credentials, "PATH": f"{fake_bin}:{os.environ['PATH']}",
                   "COMMAND_LOG": str(root / "commands.jsonl"), "RUNNER_TEMP": str(root),
                   "GITHUB_REF_NAME": "v0.1.5", "PLATFORM": "macos-arm64",
                   "MACOS_SIGNING_IDENTITY": identity, "IMPORTED_IDENTITY": identity,
                   "TEST_HOST": "aarch64-apple-darwin"}

            def run(command, overrides=None):
                shutil.rmtree(root / "dist", ignore_errors=True)
                (root / "commands.jsonl").write_text("")
                result = subprocess.run(command, cwd=root, env={**env, **(overrides or {})},
                                        capture_output=True, text=True)
                calls = [json.loads(line) for line in (root / "commands.jsonl").read_text().splitlines()]
                return result, calls

            for missing in credentials:
                result, calls = run(["bash", "-e", "-o", "pipefail", "-c", script], {missing: ""})
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"Missing repository secret: {missing}", result.stdout)
                self.assertEqual(calls, [])

            for host in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
                result, calls = run(["bash", "-e", "-o", "pipefail", "-c", script], {"TEST_HOST": host})
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual((root / "target/release/SSH Tunnel Plus.app/Contents/Info.plist").stat().st_mode & 0o777, 0o644)
                signing = next(call for call in calls if call[:2] == ["codesign", "--force"])
                self.assertIn(identity, signing)
                self.assertIn("--timestamp=none", signing)
                self.assertIn("--requirements", signing)
                requirement = next(call for call in calls if "--test-requirement" in call)
                self.assertEqual(requirement[-2], f'=identifier "local.reverse-ssh-tunnel-with-server" and certificate leaf = H"{identity}"')
                self.assertFalse(any(call[0] in ("xcrun", "spctl") for call in calls))
                verify = max(i for i, call in enumerate(calls) if call[:2] == ["codesign", "--verify"])
                package = next(i for i, call in enumerate(calls) if call[0] == "ditto")
                self.assertLess(verify, package)
                self.assertEqual(calls[-1][:2], ["security", "delete-keychain"])
                self.assertTrue(list((root / "dist").glob("*.zip")))

            for failure in ({"FAIL_TOOL": "codesign"}, {"FAIL_VERIFY": "1"},
                            {"IMPORTED_IDENTITY": "0" * 40}, {"FAIL_TOOL": "security"}):
                result, calls = run(["bash", "-e", "-o", "pipefail", "-c", script], failure)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((root / "dist").exists())
                self.assertEqual(calls[-1][:2], ["security", "delete-keychain"])

            result, calls = run(["sh", "build.sh"], {"MACOS_SIGNING_IDENTITY": ""})
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(any(call[:4] == ["codesign", "--force", "--sign", "-"] for call in calls))
            for invalid in ({"MACOS_SIGNING_IDENTITY": "-"}, {"MACOS_SIGNING_IDENTITY": "0" * 40}):
                result, calls = run(["sh", "build.sh"], invalid)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(any(call[0] == "rustup" for call in calls))
            for invalid_pin in ("", "not a fingerprint", "ABCD"):
                pin.write_text(invalid_pin)
                result, calls = run(["bash", "-e", "-o", "pipefail", "-c", script])
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((root / "dist").exists())

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

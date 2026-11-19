#!/usr/bin/env python3
"""Exercise release workflow shell blocks without publishing or using the network."""
import os
from pathlib import Path
import subprocess
import unittest


WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/release.yml"


def run_step(name, mock, **env):
    lines = WORKFLOW.read_text().splitlines()
    start = lines.index(f"      - name: {name}")
    run = lines.index("        run: |", start) + 1
    body = []
    for line in lines[run:]:
        if line and not line.startswith("          "):
            break
        body.append(line[10:])
    return subprocess.run(
        ["bash", "-eu", "-c", mock + "\n" + "\n".join(body)],
        env={**os.environ, **env}, capture_output=True, text=True, timeout=10,
    )


class ReleaseWorkflowTests(unittest.TestCase):
    def test_alpha_is_latest_normal_release(self):
        result = run_step(
            "Publish GitHub Release", 'gh() { printf "%s\\n" "$@"; }',
            GITHUB_REF_NAME="v0.1.0-alpha.44",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        args = result.stdout.splitlines()
        self.assertNotIn("--prerelease", args)
        self.assertIn("--latest", args)
        self.assertIn("OpsBox alpha 44", args)

    def test_other_prereleases_stay_prereleases(self):
        for tag in ("v0.1.0-beta.1", "v0.1.0-rc.1", "v0.2.0-alpha.1"):
            with self.subTest(tag=tag):
                result = run_step(
                    "Publish GitHub Release", 'gh() { printf "%s\\n" "$@"; }',
                    GITHUB_REF_NAME=tag,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                args = result.stdout.splitlines()
                self.assertIn("--prerelease", args)
                self.assertNotIn("--latest", args)

    def test_latest_installer_check_requires_http_200(self):
        for status in ("200", "404"):
            with self.subTest(status=status):
                result = run_step(
                    "Check latest installer URL",
                    'curl() { printf "%s" "$STATUS"; }',
                    STATUS=status, GITHUB_REPOSITORY="khangdev20/opsbox",
                )
                self.assertEqual(result.returncode == 0, status == "200", result.stderr)


if __name__ == "__main__":
    unittest.main()

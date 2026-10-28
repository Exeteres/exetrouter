#!/usr/bin/env python3
"""Offline regression checks for the publication allowlist and redacted failures."""
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check-publication.py")
spec = importlib.util.spec_from_file_location("publication", SCRIPT)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class PublicationTests(unittest.TestCase):
    def test_desktop_package_is_narrowly_allowlisted(self):
        for name in ["package.json", "package-lock.json", "README.md", "src/gui.tsx", "src/window.css", "tests/catalog.test.ts"]:
            self.assertTrue(audit.allowed_path(f"clients/opencode-desktop/{name}"))
        for name in ["clients/other/index.ts", "clients/opencode-desktop/dist/gui-experimental.cjs", "clients/opencode-desktop/node_modules/package/index.ts", "clients/opencode-desktop/src/private.key", "clients/opencode-desktop/.env", "clients/opencode-desktop/state.sqlite"]:
            self.assertFalse(audit.allowed_path(name))

    def check_file(self, name, text):
        with tempfile.TemporaryDirectory(prefix="exetrouter-publication-") as temp:
            path = Path(temp) / name
            path.parent.mkdir(parents=True)
            path.write_text(text)
            return subprocess.run([sys.executable, str(SCRIPT), "--root", temp], capture_output=True, text=True)

    def test_lockfile_funding_exception_is_scoped_and_credentials_still_fail(self):
        lock = "clients/opencode-desktop/package-lock.json"
        funding = "https://" + "opencollective.com/example"
        self.assertEqual(self.check_file(lock, '{"funding":"' + funding + '"}').returncode, 0)
        self.assertNotEqual(self.check_file("docs/example.md", funding).returncode, 0)
        secret = "exr_tok_" + "0" * 16 + "_" + "a" * 64
        result = self.check_file(lock, '{"secret":"' + secret + '"}')
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(secret, result.stderr)

    def test_unreviewed_endpoints_fail_without_echoing_the_url(self):
        url = "https://" + "unreviewed-endpoint.invalid-domain/v1"
        result = self.check_file("clients/opencode-desktop/src/example.ts", url)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(url, result.stderr)


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Offline tests for the bundled image helper; only synthetic loopback requests."""
import base64
import importlib.util
import json
import os
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import subprocess
import sys
import tempfile
import threading
import unittest

sys.dont_write_bytecode = True
HELPER = Path(__file__).resolve().parents[1] / "skills/exr/scripts/image.py"
spec = importlib.util.spec_from_file_location("exr_image", HELPER)
image = importlib.util.module_from_spec(spec)
spec.loader.exec_module(image)
PNG = b"\x89PNG\r\n\x1a\nsynthetic-image-bytes"


class ImageHelperTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = Path(self.dir.name) / "image.png"
        self.calls = []
        self.status = 200
        self.reply = json.dumps({"created": 123, "data": [{"b64_json": base64.b64encode(PNG).decode()}]}).encode()
        test = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                test.calls.append((self.path, dict(self.headers), json.loads(self.rfile.read(int(self.headers["Content-Length"])))))
                self.send_response(test.status)
                if test.status == 302:
                    self.send_header("Location", "/must-not-follow")
                self.end_headers()
                self.wfile.write(test.reply)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever)
        self.thread.start()
        self.base = f"http://127.0.0.1:{self.server.server_port}/v1"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.dir.cleanup()

    def generate(self, **kwargs):
        return image.generate(self.base, "synthetic-router-bearer", "synthetic-private-prompt", self.path, **kwargs)

    def test_generate_creates_private_image_without_writing_json_or_overwriting(self):
        result = self.generate()
        self.assertEqual(result, {"path": str(self.path), "mime_type": "image/png"})
        self.assertEqual(self.path.read_bytes(), PNG)
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.calls[0][0], "/v1/images/generations")
        self.assertEqual(self.calls[0][2]["model"], "gpt-image-2")
        with self.assertRaises(ValueError):
            self.generate()
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(list(self.path.parent.iterdir()), [self.path])

    def test_edit_embeds_references_without_uploads(self):
        reference = self.path.parent / "reference.png"
        reference.write_bytes(PNG)
        self.generate(references=[reference])
        self.assertEqual(self.calls[0][0], "/v1/images/edits")
        self.assertEqual(self.calls[0][2]["images"], [{"image_url": "data:image/png;base64," + base64.b64encode(PNG).decode()}])

    def test_http_errors_and_redirects_never_retry_echo_body_or_create_output(self):
        for status in [302, 400, 401, 429, 500]:
            with self.subTest(status=status):
                self.status = status
                self.reply = b"synthetic-private-upstream-error"
                with self.assertRaisesRegex(ValueError, f"HTTP {status}") as caught:
                    self.generate()
                self.assertNotIn("private", str(caught.exception))
                self.assertFalse(self.path.exists())
        self.assertEqual(len(self.calls), 5)

    def test_invalid_responses_never_create_output(self):
        for reply in [b"not-json", b"[]", b"null", b'{"data":[]}', b'{"data":[{"b64_json":"!"}]}', b'{"data":[{"b64_json":"dGV4dA=="}]}']:
            with self.subTest(reply=reply):
                self.reply = reply
                with self.assertRaises(ValueError):
                    self.generate()
                self.assertFalse(self.path.exists())

    def test_insecure_urls_and_missing_bearer_are_rejected_before_submission(self):
        for base in ["http://example.com/v1", "https://token@example.com/v1", "https://example.com/v1?token=secret", "https://example.com/v1#fragment"]:
            with self.assertRaises(ValueError):
                image.generate(base, "synthetic", "prompt", self.path)
        with self.assertRaises(ValueError):
            image.generate(self.base, "", "prompt", self.path)
        self.assertEqual(self.calls, [])

    def test_cli_uses_stdin_and_environment_without_printing_prompt_or_bearer(self):
        env = {**os.environ, "EXETROUTER_BASE_URL": self.base, "EXETROUTER_TOKEN": "synthetic-router-bearer"}
        result = subprocess.run([sys.executable, str(HELPER), "--output", str(self.path)], input="synthetic-private-prompt", text=True, capture_output=True, env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["path"], str(self.path))
        self.assertNotIn("synthetic-", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

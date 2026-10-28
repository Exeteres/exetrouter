#!/usr/bin/env python3
"""Generate/edit one image through ExetRouter. No retries or credential output."""

import argparse
import base64
import binascii
import json
import os
from pathlib import Path
import sys
import urllib.error
import urllib.parse
import urllib.request
import ipaddress

MAX_RESPONSE = 48 * 1024 * 1024


class ImageError(ValueError):
    """Fixed, content-free diagnostic safe for CLI output."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def image_type(data):
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    if data.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    if data.startswith(b"RIFF") and data[8:12] == b"WEBP":
        return "image/webp"
    raise ImageError("Expected a PNG, JPEG or WebP image")


def endpoint(base, edit):
    url = urllib.parse.urlsplit(base)
    try:
        local = url.hostname == "localhost" or ipaddress.ip_address(url.hostname).is_loopback
    except ValueError:
        local = False
    if (url.scheme != "https" and not (url.scheme == "http" and local)) or not url.hostname or url.username or url.password or url.query or url.fragment:
        raise ImageError("Use an HTTPS API base URL, or loopback HTTP, without credentials/query/fragment")
    return base.rstrip("/") + ("/images/edits" if edit else "/images/generations")


def generate(base, token, prompt, output, references=(), quality="auto", size="auto", background="auto"):
    if not token or any(c in token for c in "\r\n"):
        raise ImageError("Set EXETROUTER_TOKEN to an existing router bearer")
    if not prompt.strip():
        raise ImageError("Prompt must not be empty")
    if len(references) > 5:
        raise ImageError("At most five reference images are supported")
    output = Path(output).expanduser().absolute()
    if output.exists() or output.is_symlink():
        raise ImageError("Output already exists; choose a new path")
    if not output.parent.is_dir():
        raise ImageError("Output directory does not exist")
    body = {"model": "gpt-image-2", "prompt": prompt, "quality": quality, "size": size, "background": background}
    if references:
        images = []
        for reference in references:
            data = Path(reference).expanduser().read_bytes()
            images.append({"image_url": f"data:{image_type(data)};base64,{base64.b64encode(data).decode('ascii')}"})
        body["images"] = images
    request = urllib.request.Request(endpoint(base, bool(references)), data=json.dumps(body).encode(), headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json", "Accept": "application/json"}, method="POST")
    opener = urllib.request.build_opener(NoRedirect)
    try:
        with opener.open(request, timeout=900) as response:
            raw = response.read(MAX_RESPONSE + 1)
    except urllib.error.HTTPError as error:
        # Do not read, echo or persist an arbitrary error response/URL/headers.
        error.close()
        raise ImageError(f"Image request rejected (HTTP {error.code}); not retried") from None
    except (urllib.error.URLError, TimeoutError, OSError):
        raise ImageError("Image request interrupted; outcome unknown, not retried") from None
    if len(raw) > MAX_RESPONSE:
        raise ImageError("Image response exceeded the output budget; not retried")
    try:
        value = json.loads(raw)
        if not isinstance(value, dict) or value.get("error") or not isinstance(value.get("data"), list) or len(value["data"]) != 1 or not isinstance(value["data"][0], dict) or not isinstance(value["data"][0].get("b64_json"), str):
            raise ValueError
        data = base64.b64decode(value["data"][0]["b64_json"], validate=True)
        mime = image_type(data)
    except (ValueError, TypeError, KeyError, binascii.Error):
        raise ImageError("Invalid image response; not retried") from None
    # Exclusive creation prevents overwriting existing work or following an
    # output symlink. Only an incomplete file created by this call is removed.
    fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "wb") as target:
            target.write(data)
    except BaseException:
        output.unlink()
        raise
    return {"path": str(output), "mime_type": mime}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, help="New image file; existing files are never replaced")
    parser.add_argument("--reference", action="append", default=[], help="Local PNG/JPEG/WebP; repeat up to five times for editing")
    parser.add_argument("--quality", choices=["auto", "low", "medium", "high"], default="auto")
    parser.add_argument("--background", choices=["auto", "transparent", "opaque"], default="auto")
    parser.add_argument("--size", default="auto")
    args = parser.parse_args()
    try:
        # Prompt on stdin and bearer in the environment, never process arguments.
        result = generate(os.environ.get("EXETROUTER_BASE_URL", "http://127.0.0.1:8787/v1"), os.environ.get("EXETROUTER_TOKEN", ""), sys.stdin.read(), args.output, args.reference, args.quality, args.size, args.background)
    except ImageError as error:
        print(str(error), file=sys.stderr)
        return 1
    except (ValueError, OSError):
        print("Image generation failed; check configuration, output path and API availability. No automatic retry was attempted.", file=sys.stderr)
        return 1
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())

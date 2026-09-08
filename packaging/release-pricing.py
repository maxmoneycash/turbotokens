#!/usr/bin/env python3
"""Share one immutable LiteLLM pricing snapshot across a release's native builds."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import urllib.error
import urllib.request

FILENAME = "model_prices_and_context_window.json"
MANIFEST = "release-inputs.json"
LATEST_COMMIT = "https://api.github.com/repos/BerriAI/litellm/commits/main"
MAX_BYTES = 16 * 1024 * 1024


def commit_sha(value):
    if not re.fullmatch(r"[0-9a-f]{40}", value):
        raise ValueError("expected a full lowercase Git commit SHA")
    return value


def pricing_url(commit):
    return "https://raw.githubusercontent.com/BerriAI/litellm/" + commit_sha(commit) + "/" + FILENAME


class NoAuthenticatedRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        raise urllib.error.HTTPError(
            request.full_url, code, "authenticated commit lookup must not redirect",
            headers, response)


def download(url):
    headers = {"User-Agent": "turbotokens-release"}
    token = os.environ.get("GH_TOKEN") if url == LATEST_COMMIT else None
    if token:
        headers["Authorization"] = "Bearer " + token
        open_request = urllib.request.build_opener(NoAuthenticatedRedirect).open
    else:
        open_request = urllib.request.urlopen
    request = urllib.request.Request(url, headers=headers)
    with open_request(request, timeout=30) as response:
        if not response.geturl().startswith("https://"):
            raise ValueError("pricing download redirected outside HTTPS")
        data = response.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise ValueError("pricing response exceeds 16 MiB")
    return data


def check_json(data):
    value = json.loads(data)
    if not isinstance(value, dict) or not value:
        raise ValueError("pricing snapshot must be a nonempty JSON object")


def prepare(directory, source_sha, commit=""):
    source_sha = commit_sha(source_sha)
    if not commit:
        commit = json.loads(download(LATEST_COMMIT))["sha"]
    commit = commit_sha(commit)
    url = pricing_url(commit)
    data = download(url)
    check_json(data)
    manifest = {
        "source_sha": source_sha,
        "pricing": {
            "source_commit": commit, "url": url,
            "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data),
        },
    }
    directory.mkdir(parents=True, exist_ok=True)
    if any(directory.iterdir()):
        raise ValueError("release input directory must be empty")
    (directory / FILENAME).write_bytes(data)
    (directory / MANIFEST).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest


def verify(directory, source_sha, github_env=None):
    source_sha = commit_sha(source_sha)
    manifest = json.loads((directory / MANIFEST).read_text(encoding="utf-8"))
    if manifest["source_sha"] != source_sha:
        raise ValueError("release inputs belong to another source revision")
    pricing = manifest["pricing"]
    if pricing["url"] != pricing_url(pricing["source_commit"]):
        raise ValueError("pricing URL does not match its immutable commit")
    data = (directory / FILENAME).read_bytes()
    if len(data) > MAX_BYTES or len(data) != pricing["bytes"]:
        raise ValueError("pricing snapshot size differs")
    if hashlib.sha256(data).hexdigest() != pricing["sha256"]:
        raise ValueError("pricing snapshot checksum differs")
    check_json(data)
    snapshot = str((directory / FILENAME).resolve())
    if github_env is not None:
        if "\n" in snapshot or "\r" in snapshot:
            raise ValueError("pricing path contains a newline")
        with github_env.open("a", encoding="utf-8") as output:
            output.write("TURBOTOKENS_PRICING_JSON_PATH=" + snapshot + "\n")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    for name in ("prepare", "verify"):
        command = subcommands.add_parser(name)
        command.add_argument("--directory", type=Path, required=True)
        command.add_argument("--source-sha", required=True)
        if name == "prepare":
            command.add_argument("--commit", default="")
        else:
            command.add_argument("--github-env", type=Path)
    args = parser.parse_args()
    if args.command == "prepare":
        manifest = prepare(args.directory, args.source_sha, args.commit)
    else:
        manifest = verify(args.directory, args.source_sha, args.github_env)
    print(json.dumps(manifest))


if __name__ == "__main__":
    main()

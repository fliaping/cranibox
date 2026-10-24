#!/usr/bin/env python3
"""Publish verified release files to GitCode; never overwrite different assets."""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import sys
import time
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode, urlsplit
from urllib.request import Request, urlopen

from release_common import check_versions, sha256, verified_assets

API = "https://api.gitcode.com/api/v5"


class GitCode:
    def __init__(self, repository, token):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise ValueError("GitCode repository must be owner/repository.")
        if not token:
            raise ValueError("Set GITCODE_TOKEN to a token with write access to the release repository.")
        self.path = "/repos/" + "/".join(quote(part, safe="") for part in repository.split("/"))
        self.token = token

    def request(self, method, suffix, data=None, missing_ok=False, **params):
        params["access_token"] = self.token
        url = API + self.path + suffix + "?" + urlencode(params)
        payload = json.dumps(data).encode() if data is not None else None
        request = Request(url, data=payload, method=method, headers={"Content-Type": "application/json"})
        for attempt in range(3):
            try:
                with urlopen(request, timeout=90) as response:
                    result = json.load(response)
                if isinstance(result, dict) and result.get("success") is False:
                    raise RuntimeError(f"GitCode rejected {method} {suffix}.")
                return result
            except HTTPError as error:
                if missing_ok and error.code == 404:
                    return None
                if method == "GET" and (error.code == 429 or error.code >= 500) and attempt < 2:
                    time.sleep(2 ** attempt)
                    continue
                # Never print a signed URL, the token in the query, or raw API bodies.
                raise RuntimeError(f"GitCode {method} {suffix}: HTTP {error.code}.") from None
            except (URLError, TimeoutError, OSError):
                if method == "GET" and attempt < 2:
                    time.sleep(2 ** attempt)
                    continue
                raise RuntimeError(f"GitCode {method} {suffix}: network request failed.") from None
            except json.JSONDecodeError:
                raise RuntimeError(f"GitCode {method} {suffix}: invalid JSON response.") from None

    def release(self, tag):
        return self.request("GET", f"/releases/tags/{quote(tag, safe='')}", missing_ok=True)

    def download_digest(self, tag, filename):
        # No publisher token: verify public downloads work as well as uploaded data.
        suffix = f"/releases/{quote(tag, safe='')}/attach_files/{quote(filename, safe='')}/download"
        try:
            with urlopen(API + self.path + suffix, timeout=120) as response:
                if response.headers.get_content_type() == "text/html":
                    raise RuntimeError(f"Public download returned HTML instead of {filename}.")
                return hashlib.file_digest(response, "sha256").hexdigest()
        except (URLError, TimeoutError, OSError):
            raise RuntimeError(f"Could not verify public GitCode download of {filename}.") from None

    def upload(self, tag, file):
        result = self.request("GET", f"/releases/{quote(tag, safe='')}/upload_url", file_name=file.name)
        url = urlsplit(result["url"])
        if url.scheme != "https" or not url.hostname or url.username or url.password:
            raise RuntimeError("GitCode returned an invalid HTTPS upload address.")
        # Only the headers returned by GitCode go to storage, never GITCODE_TOKEN.
        headers = dict(result["headers"])
        headers["Content-Length"] = str(file.stat().st_size)
        connection = http.client.HTTPSConnection(url.hostname, url.port or 443, timeout=180)
        try:
            connection.putrequest("PUT", url.path + ("?" + url.query if url.query else ""))
            for name, value in headers.items():
                connection.putheader(name, str(value))
            connection.endheaders()
            with file.open("rb") as stream:
                while block := stream.read(1024 * 1024):
                    connection.send(block)
            response = connection.getresponse()
            body = response.read()
            if not 200 <= response.status < 300:
                raise RuntimeError(f"Upload of {file.name} failed: HTTP {response.status}.")
            if body:
                try:
                    callback = json.loads(body)
                except (json.JSONDecodeError, UnicodeDecodeError):
                    callback = None
                if isinstance(callback, dict) and callback.get("success") is False:
                    raise RuntimeError(f"GitCode did not register the upload of {file.name}.")
        except (OSError, http.client.HTTPException):
            raise RuntimeError(f"Upload of {file.name} failed; rerun to resume after verifying existing files.") from None
        finally:
            connection.close()


def publish(client, tag, assets, notes, prerelease=False):
    files = verified_assets(assets)
    release = client.release(tag)
    payload = {"name": f"Cranibox {tag}", "body": notes}
    if release is None:
        # Uploads can fail. Keep an incomplete new release out of the stable feed.
        client.request("POST", "/releases", {**payload, "tag_name": tag, "release_status": "pre"})
        release = client.release(tag)
    if not isinstance(release, dict) or release.get("tag_name") != tag:
        raise RuntimeError("GitCode returned an unexpected release.")
    existing = {asset["name"]: asset for asset in release.get("assets", [])}
    for file in files:
        if file.name in existing:
            # A retry is safe only when the actual public file is byte-for-byte identical.
            if client.download_digest(tag, file.name) != sha256(file):
                raise ValueError(f"GitCode already has different contents for {file.name}; refusing to overwrite.")
            print(f"Already uploaded and verified {file.name}", flush=True)
            continue
        client.upload(tag, file)
        for attempt in range(4):
            visible = client.release(tag)
            if any(asset["name"] == file.name for asset in (visible or {}).get("assets", [])):
                break
            if attempt == 3:
                raise RuntimeError(f"GitCode did not list {file.name} after upload.")
            time.sleep(2 ** attempt)
        if client.download_digest(tag, file.name) != sha256(file):
            raise RuntimeError(f"Public GitCode download of {file.name} failed checksum verification.")
        print(f"Uploaded and verified {file.name}", flush=True)
    # Replaying an old stable tag makes it latest again; the manual workflow and
    # release guide call this out. Never move a published tag to different contents.
    client.request("PATCH", f"/releases/{quote(tag, safe='')}",
                   {**payload, "release_status": "pre" if prerelease else "latest"})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--notes-file", type=Path, required=True)
    parser.add_argument("--prerelease", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    try:
        version = check_versions(args.tag)
        files = verified_assets(args.assets)
        notes = args.notes_file.read_text()
        if args.dry_run:
            for file in files:
                print(f"Would publish {file.name} to {args.repository} {args.tag}")
            return
        publish(GitCode(args.repository, os.environ.get("GITCODE_TOKEN")), args.tag,
                args.assets, notes, args.prerelease or "-" in version)
    except (OSError, ValueError, RuntimeError, KeyError) as error:
        # Inputs and API payloads can include secrets; unexpected payload keys are
        # reported without dumping their values.
        print(f"publish-gitcode: {error}", file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()

"""Shared, fail-closed parsing of npm stage metadata (stdlib only)."""
import json
import os
import re
import subprocess

from release_common import PLATFORMS, validate_version

NPM = "npm.cmd" if os.name == "nt" else "npm"
REGISTRY = "https://registry.npmjs.org"


def package_names(name):
    if not re.fullmatch(r"@[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*", name):
        raise ValueError("The npm package must be a scoped lowercase name, e.g. @fliaping/cranibox.")
    return [f"{name}-{target}" for target in PLATFORMS] + [name]


def stage_json(action, value):
    result = subprocess.run([NPM, "stage", action, value, "--json", "--registry", REGISTRY],
                            capture_output=True, text=True)
    if result.returncode:
        # A new package may not exist in the public registry yet. Only an explicit
        # not-found response to list means no stages; auth/network errors must fail.
        try:
            error = json.loads(result.stdout).get("error", {})
        except (ValueError, AttributeError):
            error = {}
        if action == "list" and isinstance(error, dict) and error.get("code") == "E404":
            return []
        raise RuntimeError(f"npm stage {action} failed for {value} (exit {result.returncode}).")
    try:
        return json.loads(result.stdout)
    except ValueError:
        raise ValueError(f"Invalid JSON from npm stage {action} for {value}.") from None


def stage_records(data):
    if isinstance(data, dict):
        for key in ("stages", "results", "data"):
            if key in data:
                return stage_records(data[key])
        if any(key in data for key in ("id", "stageId", "stage_id")):
            data = [data]
        elif data and all(re.fullmatch(r"@[^/\s]+/\S+", str(key)) and isinstance(value, (dict, list))
                          for key, value in data.items()):
            # npm 12 keys results by package name.
            data = [item for value in data.values() for item in (value if isinstance(value, list) else [value])]
    if not isinstance(data, list) or any(not isinstance(item, dict) for item in data):
        raise ValueError("Unrecognized npm stage metadata; refusing to assume no staged versions.")
    return data


def normalize_stage(item):
    package = item.get("package", {})
    manifest = item.get("manifest", {})
    if not isinstance(manifest, dict):
        raise ValueError("Invalid staged manifest.")
    if isinstance(package, str):
        package = {"name": package}
    if not isinstance(package, dict):
        raise ValueError("Invalid staged package.")
    dist = item.get("dist") or package.get("dist") or manifest.get("dist") or {}
    if not isinstance(dist, dict):
        raise ValueError("Invalid staged integrity metadata.")
    result = {
        "id": item.get("id") or item.get("stageId") or item.get("stage_id"),
        "name": item.get("name") or item.get("packageName") or package.get("name") or manifest.get("name"),
        "version": item.get("version") or item.get("packageVersion") or package.get("version") or manifest.get("version"),
        "tag": item.get("tag") or item.get("distTag") or item.get("dist-tag"),
        "integrity": item.get("integrity") or dist.get("integrity"),
    }
    for key, value in result.items():
        if value is not None and (not isinstance(value, str) or not value or any(c.isspace() for c in value)):
            raise ValueError(f"Invalid staged {key}.")
    if not result["id"] or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", result["id"]):
        raise ValueError("Missing or unsafe npm stage ID.")
    return result


def list_stages(name, require_integrity=False, version=None):
    """With version, stages of other versions are skipped before their metadata is required:
    an approved or superseded stage need not report integrity to publish a new version."""
    stages = []
    seen = set()
    for item in stage_records(stage_json("list", name)):
        stage = normalize_stage(item)
        # Never act on another package, even if npm returns an unfiltered list.
        if stage["name"] and stage["name"] != name:
            continue
        if version and stage["version"] and stage["version"] != version:
            continue
        required = ("name", "version", "tag") + (("integrity",) if require_integrity else ())
        if any(not stage[key] for key in required):
            details = stage_records(stage_json("view", stage["id"]))
            if len(details) != 1:
                raise ValueError("Expected exactly one stage from npm stage view.")
            detail = normalize_stage(details[0])
            for key, value in detail.items():
                if stage[key] and value and stage[key] != value:
                    raise ValueError(f"Conflicting staged {key} for {stage['id']}.")
                stage[key] = value or stage[key]
        if stage["name"] != name:
            raise ValueError(f"Cannot verify staged package identity for {stage['id']}.")
        if version and stage["version"] and stage["version"] != version:
            continue
        if any(not stage[key] for key in required):
            raise ValueError(f"Cannot verify staged metadata for {stage['id']}; inspect npm stage view before retrying.")
        validate_version(stage["version"])
        if stage["id"] in seen:
            raise ValueError(f"Duplicate npm stage ID: {stage['id']}.")
        seen.add(stage["id"])
        stages.append(stage)
    return stages

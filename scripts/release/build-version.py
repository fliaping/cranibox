#!/usr/bin/env python3
"""Print the version this CI build is published as; CI exports it as CRANIBOX_BUILD_VERSION.

Tags (refs/tags/vX.Y.Z) build that version; every other build is
`<next patch of crates/cranibox's Cargo version>-dev.<GITHUB_RUN_NUMBER>`. The binaries read
CRANIBOX_BUILD_VERSION at compile time (`option_env!`), so `cranibox --version` matches the
npm package and Info.plist without editing Cargo.toml. Local builds without it report the
Cargo version.
"""
import os
from pathlib import Path
import sys
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release_common import ROOT, build_version  # noqa: E402


def main():
    source = tomllib.loads((ROOT / "crates/cranibox/Cargo.toml").read_text())["package"]["version"]
    print(build_version(os.environ.get("GITHUB_REF", ""), os.environ.get("GITHUB_REF_NAME", ""),
                        os.environ.get("GITHUB_RUN_NUMBER", ""), source))


if __name__ == "__main__":
    main()

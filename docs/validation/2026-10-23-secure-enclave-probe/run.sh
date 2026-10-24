#!/bin/sh
# Build the probe as an ad-hoc signed binary (like the current Cranibox packaging) and run it.
set -eu
cd "$(dirname "$0")"
swiftc -O probe.swift -o probe
echo "== signature =="
codesign -dv probe 2>&1 | grep -E "Signature|TeamIdentifier|flags" || true
echo "== run 1: create the key, then press Touch ID =="
./probe || true
echo "== run 2: reload the key file, then press CANCEL on Touch ID (should fail) =="
./probe || true
echo "== run 3: reload the key file, then press Touch ID =="
./probe || true

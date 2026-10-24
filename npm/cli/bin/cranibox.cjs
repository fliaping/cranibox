#!/usr/bin/env node
'use strict';

const { spawn } = require('node:child_process');
const { existsSync } = require('node:fs');
const path = require('node:path');

function binaryPath(manifest, platform = process.platform, arch = process.arch, report = process.report) {
  let target = `${platform}-${arch}`;
  if (platform === 'linux') {
    // The Linux release is linked against glibc, not Alpine's musl.
    if (!report?.getReport().header.glibcVersionRuntime) {
      throw new Error('This Cranibox package requires glibc Linux; musl/Alpine is not supported.');
    }
    target += '-gnu';
  }
  const packageName = manifest.craniboxPlatforms?.[target];
  if (!packageName) throw new Error(`Cranibox does not provide an npm binary for ${target}.`);
  let packageFile;
  try {
    packageFile = require.resolve(`${packageName}/package.json`);
  } catch {
    throw new Error(`Missing ${packageName}@${manifest.version}. Reinstall ${manifest.name}@${manifest.version} with optional dependencies enabled (--include=optional).`);
  }
  const installed = require(packageFile);
  if (installed.version !== manifest.version) {
    throw new Error(`Version mismatch: ${packageName}@${installed.version}; expected ${manifest.version}. Reinstall ${manifest.name}@${manifest.version}.`);
  }
  const binary = path.join(path.dirname(packageFile), 'bin', platform === 'win32' ? 'cranibox.exe' : 'cranibox');
  if (!existsSync(binary)) throw new Error(`The binary is missing from ${packageName}. Reinstall ${manifest.name}@${manifest.version}.`);
  return binary;
}

function main() {
  let binary;
  try {
    binary = binaryPath(require('../package.json'));
  } catch (error) {
    console.error(`cranibox: ${error.message}`);
    process.exitCode = 1;
    return;
  }
  const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit', windowsHide: false });
  // Signals from a terminal reach the whole foreground process group. Signals sent
  // directly to this wrapper must also reach the native process (MCP/CI teardown).
  const handlers = new Map();
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
    const handler = () => child.kill(signal);
    handlers.set(signal, handler);
    process.on(signal, handler);
  }
  child.once('error', error => {
    for (const [signal, handler] of handlers) process.removeListener(signal, handler);
    console.error(`cranibox: could not start the native program: ${error.message}`);
    process.exitCode = 1;
  });
  child.once('exit', (code, signal) => {
    for (const [name, handler] of handlers) process.removeListener(name, handler);
    if (signal && process.platform !== 'win32') process.kill(process.pid, signal);
    else process.exitCode = code ?? 1;
  });
}

if (require.main === module) main();
module.exports = { binaryPath };

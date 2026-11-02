# Cranibox CLI

Run a Cranibox node on your device and connect it to your other devices and agents.
Requires Node.js 22 or newer. Supported binaries: macOS Apple Silicon, Windows x64,
and Linux x64 with glibc (built on the workflow's Ubuntu runner). Intel Macs, Linux arm64 and
musl/Alpine are not included.

```sh
npm install -g @fliaping/cranibox
cranibox --help
cranibox --version
```

In mainland China, install or upgrade through the npm mirror:

```sh
npm install -g @fliaping/cranibox@latest --registry=https://registry.npmmirror.com
```

The native executable is included in a platform package pinned to the exact CLI
version. There are no install scripts, runtime downloads or GitHub downloads.
Keep npm optional dependencies enabled. If you use a mirror, the CLI and its
platform packages must all have synced before you can install a new version.

Development builds from the main branch are published under the `dev` tag:

```sh
npm install -g @fliaping/cranibox@dev
```

```sh
npx @fliaping/cranibox@latest --help
npm install -g @fliaping/cranibox@1.1.0  # select a specific version
npm uninstall -g @fliaping/cranibox
```

Use npm to update npm-installed copies. An already running Cranibox process must
be restarted after an upgrade to use the new executable. Devices that run a managed
runtime update themselves with `cranibox update apply latest` (or `next` / `dev`)
and do not need npm. Device identity and
configuration live outside this package and are retained on upgrades/uninstall.

See the [project documentation](https://github.com/fliaping/cranibox#readme) for
pairing, permissions, the web interface and MCP setup. Standalone executables and
desktop installers are available in the project's Releases.

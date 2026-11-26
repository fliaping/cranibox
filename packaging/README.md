# Package manager manifests

These files install a published [GitHub Release](https://github.com/fliaping/cranibox/releases). They are not published automatically. Update versions and checksums with:

```bash
python3 scripts/release/bump-packaging.py --tag v1.2.1 --checksums SHA256SUMS
```

`--checksums` is a `SHA256SUMS` file from that release (or omit it to download the release file).

## Homebrew (#33)

The formula in `homebrew/cranibox.rb` installs the Apple Silicon CLI and checks its checksum.

It is not a tap yet. To try it locally:

```bash
brew install --formula packaging/homebrew/cranibox.rb
cranibox --version
```

For `brew install fliaping/tap/cranibox` to work, a public repository named `fliaping/homebrew-tap` must exist, with this formula at `Formula/cranibox.rb`, and each release should bump the version and `sha256`. Creating that repository, and any workflow that pushes to it, needs an explicit go-ahead.

## Scoop and winget (#34)

Scoop (CLI):

```bash
scoop install packaging/scoop/cranibox.json
```

To make `scoop install cranibox` work without a path, add a Scoop bucket (a repository or a folder of manifests) and ask people to `scoop bucket add …`, or submit the manifest to a known bucket such as `extras`. That publish step is not done here.

Winget (desktop installer `Cranibox_*_x64-setup.exe`): the manifests in `winget/` follow the winget-pkgs layout for package id `fliaping.Cranibox`. They are not in the official source until a pull request is opened against [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs). After it is accepted, `winget install fliaping.Cranibox` (or `winget install Cranibox`) should work. Check the ProductCode and installer scope against a real install before submitting, in case the NSIS script differs.

## AUR (#40)

`aur/cranibox-bin/PKGBUILD` installs the prebuilt Linux CLI (`x86_64` and `aarch64`) and verifies `SHA256SUMS`. Publishing to the Arch User Repository needs an AUR account and `git push` to `ssh://aur@aur.archlinux.org/cranibox-bin.git`. That is not done here.

Generate `.SRCINFO` on an Arch system before publishing:

```bash
cd packaging/aur/cranibox-bin
makepkg --printsrcinfo > .SRCINFO
```

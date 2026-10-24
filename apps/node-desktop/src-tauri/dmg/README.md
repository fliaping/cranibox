# DMG background

`background.png` is the original 1320×920 Retina artwork for a 660×460-point
Finder window. Do not pass it directly to the bundler: Finder displays it at
the wrong logical size and crops the installation instructions.

On macOS, `npm run build` runs `scripts/prepare-dmg.mjs` to generate
`background.generated.tiff` with 660×460 @72 dpi and 1320×920 @144 dpi
representations. This uses the built-in `sips` and `tiffutil` tools; AppKit
checks the logical size and both representations before bundling. CI repeats
the check against the background inside the mounted DMG.

Keep the source artwork exactly twice `bundle.macOS.dmg.windowSize` in
`tauri.conf.json`. The build validates this on every platform. The generated
TIFF is ignored by Git. To prepare it without building the UI, run
`node scripts/prepare-dmg.mjs` from `apps/node-desktop` on macOS.

The multi-resolution TIFF approach is also described in
[create-dmg's Retina background discussion](https://github.com/create-dmg/create-dmg/issues/176).

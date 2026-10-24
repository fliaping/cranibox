import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, readFileSync, renameSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const tauri = fileURLToPath(new URL('../src-tauri/', import.meta.url));
const config = join(tauri, 'tauri.conf.json');
const dmg = JSON.parse(readFileSync(config, 'utf8')).bundle.macOS.dmg;
const { width, height } = dmg.windowSize;
const source = join(tauri, 'dmg/background.png');
const png = readFileSync(source);
assert.equal(png.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
assert.equal(png.toString('ascii', 12, 16), 'IHDR');
assert.equal(png.readUInt32BE(16), width * 2, 'DMG artwork must be twice the window width');
assert.equal(png.readUInt32BE(20), height * 2, 'DMG artwork must be twice the window height');

// Finder does not infer Retina scale from a large PNG. Package explicit 1x/2x
// representations with the same logical size, retaining the original artwork.
if (process.platform === 'darwin') {
  const temp = mkdtempSync(join(tmpdir(), 'cranibox-dmg-'));
  const run = (command, args) => execFileSync(command, args, { stdio: 'inherit' });
  try {
    const standard = join(temp, 'background.tiff');
    const retina = join(temp, 'background@2x.tiff');
    const output = join(tauri, dmg.background);
    const tiff = join(temp, 'combined.tiff');
    run('/usr/bin/sips', ['--resampleHeightWidth', String(height), String(width),
      '-s', 'format', 'tiff', '-s', 'dpiWidth', '72', '-s', 'dpiHeight', '72', source, '--out', standard]);
    run('/usr/bin/sips', ['-s', 'format', 'tiff', '-s', 'dpiWidth', '144', '-s', 'dpiHeight', '144',
      source, '--out', retina]);
    run('/usr/bin/tiffutil', ['-cathidpicheck', standard, retina, '-out', tiff]);
    run('/usr/bin/swift', [fileURLToPath(new URL('./check-dmg-background.swift', import.meta.url)),
      tiff, config]);
    // Copy across volumes (TMPDIR may be on a different filesystem).
    copyFileSync(tiff, `${output}.tmp`);
    renameSync(`${output}.tmp`, output);
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
} else {
  console.log('DMG artwork dimensions verified; TIFF generation requires macOS.');
}

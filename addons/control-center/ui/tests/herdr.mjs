import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

// Workspace terminals on the preview host run in herdr. Without it every terminal test fails
// with "Install herdr in this workspace", so resolve it once: CRANIBOX_HERDR_BIN, then the
// repository's target/herdr/herdr, then a checksum-verified install into target/herdr.
const repository = fileURLToPath(new URL('../../../../', import.meta.url));
const installed = path.join(repository, 'target/herdr/herdr');

async function executable(file) {
  try { await fs.access(file, fs.constants.X_OK); return true; } catch { return false; }
}

export async function ensureHerdr() {
  if (process.env.CRANIBOX_HERDR_BIN) {
    const configured = path.resolve(process.env.CRANIBOX_HERDR_BIN);
    if (!await executable(configured)) throw Error('CRANIBOX_HERDR_BIN is not an executable herdr: ' + configured);
    return configured;
  }
  if (await executable(installed)) return installed;
  // Both preview servers start together; only one of them installs.
  const lock = installed + '.installing';
  await fs.mkdir(path.dirname(installed), { recursive: true });
  for (let attempt = 0; ; attempt++) {
    try { await fs.mkdir(lock); break; } catch {
      if (await executable(installed)) return installed;
      if (attempt > 1200) throw Error('Timed out waiting for another herdr install (' + lock + ')');
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  }
  try {
    if (await executable(installed)) return installed;
    console.error('Installing herdr for the terminal tests into ' + path.dirname(installed));
    const result = spawnSync('python3', [path.join(repository, 'scripts/install-herdr.py'), '--output-directory', path.dirname(installed)], { stdio: 'inherit', timeout: 300000 });
    if (result.status !== 0 || !await executable(installed)) {
      throw Error('herdr is required for the terminal tests. Run `python3 scripts/install-herdr.py --output-directory target/herdr` or set CRANIBOX_HERDR_BIN.');
    }
    return installed;
  } finally {
    await fs.rm(lock, { recursive: true, force: true });
  }
}

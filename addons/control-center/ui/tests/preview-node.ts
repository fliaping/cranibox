import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

/** NodeID of the second device that tests/preview.mjs pairs with the preview host. */
export async function previewNode(port = 19080): Promise<string> {
  const file = path.join(os.tmpdir(), 'cranibox-ui-preview-node-' + port);
  for (let attempt = 0; attempt < 300; attempt++) {
    try { return fs.readFileSync(file, 'utf8').trim(); } catch { /* The preview is still pairing. */ }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw Error('The preview device was not paired');
}

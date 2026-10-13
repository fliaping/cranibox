from pathlib import Path
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]

NODE_SCRIPT = r"""
const fs = require('fs'); const vm = require('vm');
const file = process.argv[1];
const source = fs.readFileSync(file, 'utf8');
const parse = (value) => { const u = new URL(value); return { authority: u.host, toString: () => u.toString() }; };
const vscode = { env: { language: 'en', asExternalUri: async (uri) => ({ proxied: uri.toString() }) },
  Uri: { parse }, window: { createOutputChannel: () => ({}) }, StatusBarAlignment: { Left: 1 }, ThemeColor: class {} };
const module = { exports: {} };
vm.runInNewContext(source, { module, exports: module.exports, require: (n) => n === 'vscode' ? vscode : require(n),
  __dirname: '.', __filename: file, Buffer, URL, process, setTimeout, clearTimeout, setInterval, clearInterval });
const controller = module.exports(vscode);
(async () => {
  const out = {};
  for (const url of ['https://localhost:3001', 'https://127.0.0.1:3001/', 'https://desktop.example.com/']) {
    const r = await controller.desktopExternalUri(url);
    out[url] = r.proxied ? 'proxy:' + r.proxied : r.toString();
  }
  console.log(JSON.stringify(out));
})();
"""


@unittest.skipUnless(shutil.which("node"), "node is required")
class DesktopUrlTests(unittest.TestCase):
    def test_loopback_desktop_url_uses_http_selkies_listener(self):
        result = subprocess.run(["node", "-e", NODE_SCRIPT, str(ROOT / "extensions/control-center/control-center.js")],
                                capture_output=True, text=True, check=True)
        import json
        mapping = json.loads(result.stdout)
        self.assertEqual(mapping["https://localhost:3001"], "proxy:http://localhost:3000/")
        self.assertEqual(mapping["https://127.0.0.1:3001/"], "proxy:http://localhost:3000/")
        self.assertEqual(mapping["https://desktop.example.com/"], "https://desktop.example.com/")


if __name__ == "__main__":
    unittest.main()

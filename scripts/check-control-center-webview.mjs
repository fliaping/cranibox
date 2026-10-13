import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const scriptRoot = path.dirname(fileURLToPath(import.meta.url));
const extensionFile = path.resolve(scriptRoot, '../extensions/control-center/control-center.js');
const extensionSource = fs.readFileSync(extensionFile, 'utf8');
assert.match(extensionSource, /openclaw:'OpenClaw'/, 'Control Center must render OpenClaw');
assert.ok(extensionSource.includes('function agentWebButton(a)'), 'Control Center must render Agent Web UI actions');
assert.ok(extensionSource.includes('data-action="updateOpenClaw"'), 'OpenClaw card must expose Workspace-managed update');

function loadRenderers(language) {
  const source = fs.readFileSync(extensionFile, 'utf8');
  const module = { exports: {} };
  const vscode = {
    env: { language },
    window: { createOutputChannel: () => ({}) },
    StatusBarAlignment: { Left: 1 },
    ThemeColor: class {}
  };
  vm.runInNewContext(source, {
    module,
    exports: module.exports,
    require: (name) => name === 'vscode' ? vscode : require(name),
    __dirname: path.dirname(extensionFile),
    __filename: extensionFile,
    Buffer,
    URL,
    process,
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval
  }, { filename: extensionFile });
  return module.exports(vscode);
}

function inlineScripts(html) {
  return [...html.matchAll(/<script nonce="[^"]+">([\s\S]*?)<\/script>/g)]
    .map((match) => match[1]);
}

for (const language of ['en', 'zh-cn']) {
  const renderers = loadRenderers(language);
  assert.equal(
    renderers.isSnapshotHealthy({ summary: { healthy: true }, bootstrap: { foundation_ready: false } }),
    true,
    `${language} health must follow diagnostics instead of the bootstrap marker`
  );
  assert.equal(renderers.isSnapshotHealthy({ summary: { healthy: false } }), false);
  const webview = { cspSource: 'self' };
  for (const [name, html] of [
    ['control center', renderers.controlCenterHtml(webview, false)],
    ['quick view', renderers.quickViewHtml(webview)]
  ]) {
    const scripts = inlineScripts(html);
    assert.ok(scripts.length > 0, `${language} ${name} has no inline script`);
    for (const script of scripts) {
      assert.doesNotThrow(
        () => new Function(script),
        `${language} ${name} generated invalid JavaScript`
      );
    }
    if (name === 'control center') {
      assert.match(
        html,
        /const ok=data\.summary\.healthy;/,
        `${language} Control Center health must follow diagnostics`
      );
      assert.doesNotMatch(
        html,
        /const ok=data\.summary\.healthy&&data\.bootstrap\.foundation_ready;/,
        `${language} Control Center health still depends on the bootstrap marker`
      );
    }
  }
}

console.log('Control Center generated webview scripts: ok');

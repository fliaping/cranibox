#!/usr/bin/env node
'use strict';
// React is compiled into static assets. The installed Rust server needs no Node.
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const output = path.resolve(__dirname, '../addons/control-center');
const result = spawnSync(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['run', 'build', '--prefix', path.join(output, 'ui')], { stdio: 'inherit' });
if (result.status !== 0) process.exit(result.status || 1);
for (const asset of ['web.js', 'web.css']) {
  const source = path.join(output, 'ui/dist', asset), destination = path.join(output, asset);
  if (asset === 'web.js') {
    const notice = ['herdr-web-ui.txt', 'novnc.txt'].map(name => fs.readFileSync(path.resolve(__dirname, '../docs/licenses', name), 'utf8')).join('\n\n').replace(/[ \t]+$/gm, '');
    fs.writeFileSync(destination, '/*!\n' + notice + '\n*/\n' + fs.readFileSync(source, 'utf8'));
  } else fs.copyFileSync(source, destination);
}
// The tab icon is inlined: the page serves no other static files besides its bundle.
const favicon = 'data:image/png;base64,' + fs.readFileSync(path.join(__dirname, '..', 'addons', 'control-center', 'favicon.png')).toString('base64');
for (const language of ['en', 'zh-cn']) {
  const zh = language === 'zh-cn';
  const html = '<!doctype html>\n<html lang="' + language + '"><head>\n' +
    '<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">\n' +
    // xterm's DOM renderer generates styles for cell geometry and ANSI colors.
    // Its public API cannot apply a nonce; only styles allow inline content.
    '<meta http-equiv="Content-Security-Policy" content="default-src \'none\'; script-src \'self\' \'nonce-__WORKSPACE_NONCE__\'; style-src \'self\' \'unsafe-inline\'; connect-src \'self\'; frame-src \'self\'; img-src \'self\' data:; font-src \'self\'; base-uri \'none\'; form-action \'self\';">\n' +
    '<meta name="color-scheme" content="light dark"><title>Cranibox</title>\n' +
    '<link rel="icon" type="image/png" href="' + favicon + '"><link rel="stylesheet" href="/web.css">\n' +
    '</head><body><div id="root"><p>' + (zh ? '正在加载 Cranibox…' : 'Loading Cranibox…') + '</p></div>\n' +
    '<noscript>' + (zh ? '请启用 JavaScript 后使用 Cranibox。' : 'Enable JavaScript to use Cranibox.') + '</noscript>\n' +
    // Keep the configuration alone in its script; tools read it as window.workspaceWeb=…;</script>.
    '<script nonce="__WORKSPACE_NONCE__">window.workspaceWeb=__WORKSPACE_CONFIG__;</script>\n' +
    // Preferences saved on the node win; browser storage covers servers that send none.
    '<script nonce="__WORKSPACE_NONCE__">(function(){var c=window.workspaceWeb,p=c.preferences||{},s=null,l=null;try{s=localStorage.getItem("cranibox.theme");l=localStorage.getItem("cranibox.language")}catch(e){}var t=p.theme||s;document.documentElement.dataset.theme=t==="light"||t==="dark"?t:matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light";if(!p.language&&(l==="en"||l==="zh-cn"))c.language=l;document.documentElement.lang=c.language})()</script>\n' +
    '<script nonce="__WORKSPACE_NONCE__" src="/web.js" type="module"></script>\n</body></html>\n';
  fs.writeFileSync(path.join(output, 'index.' + language + '.html'), html);
}

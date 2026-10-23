"""Opt-in check against an existing Selkies desktop, through an outbound node.

CRANIBOX_TEST_REAL_SELKIES=127.0.0.1:8082 enables this test. The viewer shows
Selkies' own page on the application origin, which joins the current desktop
session, and opens its own kdialog. Requires Node, Chromium, kdialog, the Web
UI's npm dependencies, and access to the graphical session. No compositor
restart, package installation, or persistent node configuration.
"""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import threading
import unittest

from workspace_web_harness import CORE, NODE, ROOT, WEB, Harness, free_port


def forward(port, target):
    """Same host, another port: the separate application origin of a deployment."""
    listener = socket.create_server(('127.0.0.1', port))

    def pipe(source, sink):
        try:
            while data := source.recv(65536):
                sink.sendall(data)
        except OSError:
            pass
        finally:
            for end in (source, sink):
                try:
                    end.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

    def accept():
        while True:
            try:
                client, _ = listener.accept()
            except OSError:
                return
            upstream = socket.create_connection(('127.0.0.1', target))
            for a, b in ((client, upstream), (upstream, client)):
                threading.Thread(target=pipe, args=(a, b), daemon=True).start()
    threading.Thread(target=accept, daemon=True).start()
    return listener


@unittest.skipUnless(os.environ.get('CRANIBOX_TEST_REAL_SELKIES'), 'opt in to the existing Selkies desktop')
class RealSelkiesTests(unittest.TestCase):
    def test_own_page_keyboard_reconnect_and_revocation_through_relay(self):
        for tool in ('node', 'kdialog'):
            self.assertIsNotNone(shutil.which(tool), f'Missing integration tool: {tool}')
        for binary in (CORE, NODE, WEB):
            self.assertTrue(binary.is_file(), f'Missing integration binary: {binary}')
        h = Harness(unified=True)
        self.addCleanup(h.close)
        applications = free_port()
        self.addCleanup(forward(applications, h.web_port).close)
        settings = h.root / '.config/agent-workspace/web.json'
        settings.write_text(json.dumps({'port': h.web_port, 'applicationsUrl': f'http://127.0.0.1:{applications}'}))
        h.stop(h.web)
        h.start_web()
        h.login()
        config = h.root / 'test-node/settings.json'

        def cli(*args):
            result = subprocess.run([str(NODE), '--config', str(config), 'peer', *args],
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)

        code, identity = h.post('/api/peers', {'operation': 'identity'})
        self.assertEqual(code, 200)
        cli('desktop', '--protocol', 'selkies', '--address', os.environ['CRANIBOX_TEST_REAL_SELKIES'])
        cli('grant', '--allow-unverified', '--node', identity['node_id'], '--desktop', '--output-token', str(h.root / 'desktop.token'))
        # The browser owns the dialog process and closes only that process on failure.
        # Actual typed text must reach kdialog, not a fixture or browser DOM input.
        script = r'''
const fs = require('node:fs'), {createRequire} = require('node:module'), {spawn, spawnSync} = require('node:child_process');
const options = JSON.parse(fs.readFileSync(0, 'utf8'));
const {chromium, expect} = createRequire(options.ui + '/package.json')('@playwright/test');
(async () => {
  const browser = await chromium.launch({executablePath: options.chromium, args:['--no-sandbox']});
  let dialog, page;
  try {
    page = await browser.newPage({viewport:{width:1440,height:1000}});
    const errors=[]; page.on('pageerror', error => errors.push(error.message));
    await page.goto(options.url + '/#/desktop');
    const section = page.getByRole('region',{name:'Cross-device desktops',exact:true});
    await section.getByRole('button').filter({hasText:'test-node'}).click();
    await section.getByRole('button',{name:'Connect desktop',exact:true}).click();
    const status = section.locator('.vnc-toolbar [role="status"]');
    await expect(status).toHaveText('Connected',{timeout:25000});
    await expect(section.locator('.vnc-protocol')).toHaveText('Selkies');
    const frame = page.frameLocator('iframe.vnc-frame');
    await expect.poll(async () => page.frames().find(f => f.url().includes('/cranibox-app-'))?.title(), {timeout:25000}).toBe('Selkies');
    const view = frame.locator('canvas, video').first();
    await expect(view).toBeVisible({timeout:25000});
    const picture = await view.boundingBox();
    expect(picture.width).toBeGreaterThanOrEqual(320); expect(picture.height).toBeGreaterThanOrEqual(240);
    dialog=spawn('kdialog',['--title','Cranibox desktop validation','--inputbox','Cranibox remote input test'],{stdio:['ignore','pipe','pipe'],env:{...process.env,QT_QPA_PLATFORM:'xcb'}});
    let output=''; dialog.stdout.on('data', chunk => output+=chunk);
    const ended=new Promise((resolve,reject)=>{dialog.on('error',reject);dialog.on('exit',code=>code===0?resolve(output.trim()):reject(Error('Validation dialog exited '+code)));});
    // Attach a rejection handler immediately; await the same promise after typing.
    ended.catch(()=>{});
    await page.waitForTimeout(1500);
    const text='cranibox-selkies-input-20261023';
    await view.click({position:{x:2,y:2}});
    await page.keyboard.type(text); await page.keyboard.press('Enter');
    const typed=await Promise.race([ended,new Promise((_,reject)=>setTimeout(()=>reject(Error('KDE did not receive keyboard input')),10000))]);
    expect(typed).toBe(text); dialog=undefined;
    await section.getByRole('button',{name:'Disconnect',exact:true}).click();
    await section.getByRole('button',{name:'Reconnect',exact:true}).click();
    await expect(status).toHaveText('Connected',{timeout:25000});
    await expect(frame.locator('canvas, video').first()).toBeVisible({timeout:25000});
    if(options.artifact) await section.screenshot({path:options.artifact});
    const revoke=spawn(options.node,['--config',options.config,'peer','revoke','--node',options.caller],{stdio:'ignore'});
    await new Promise((resolve,reject)=>{revoke.on('error',reject);revoke.on('exit',code=>code===0?resolve():reject(Error('Revocation failed')));});
    // Revocation closes the tunnelled page: its own API stops answering.
    const page_url = page.frames().find(f => f.url().includes('/cranibox-app-')).url();
    await expect.poll(async () => (await page.request.get(new URL('api/status', page_url).href)).status(), {timeout:5000}).not.toBe(200);
    expect(errors).toEqual([]);
    console.log(JSON.stringify({picture,keyboard:true,reconnect:true,revocation:true,pageErrors:errors}));
  } catch(error) {
    if(page && options.artifact) await page.screenshot({path:options.artifact.replace(/\.png$/,'.failed.png')}).catch(()=>{});
    if(page) console.error('Desktop error:',await page.locator('.remote-desktops [role="alert"]').allTextContents());
    throw error;
  } finally { if(dialog && dialog.exitCode===null) dialog.kill('SIGTERM'); await browser.close(); }
})().catch(error=>{console.error(error.stack);process.exitCode=1;});
'''
        result = subprocess.run(['node', '-e', script], input=json.dumps({
            'ui': str(ROOT / 'addons/control-center/ui'), 'url': f'http://127.0.0.1:{h.web_port}',
            'chromium': os.environ.get('CRANIBOX_TEST_CHROMIUM', '/usr/bin/chromium'),
            'artifact': os.environ.get('CRANIBOX_TEST_SELKIES_SCREENSHOT'),
            'node': str(NODE), 'config': str(config), 'caller': identity['node_id'],
        }), capture_output=True, text=True, timeout=100)
        self.assertEqual(result.returncode, 0, result.stderr)
        evidence = json.loads(result.stdout.strip())
        self.assertTrue(evidence['keyboard'] and evidence['reconnect'] and evidence['revocation'])
        print('Selkies evidence: ' + json.dumps(evidence), flush=True)

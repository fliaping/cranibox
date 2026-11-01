/* Cranibox desktop switch for the Selkies page.
 * The page itself (nginx) always loads. Before starting the Selkies client we
 * ask the container whether the desktop is on; when it is off we show a calm
 * "桌面已关闭" screen with a button to start it, instead of a broken viewer.
 * When it is on, a small button in the corner turns it off again. */
(function () {
  'use strict';
  var script = document.currentScript;
  var entry = script && script.getAttribute('data-entry');
  var api = new URL('cranibox-desktop/', document.baseURI).href;
  var started = false;
  var root, card, pollTimer;

  function request(path, method) {
    var controller = typeof AbortController === 'function' ? new AbortController() : null;
    var timer = controller && setTimeout(function () { controller.abort(); }, 4000);
    return fetch(api + path, {
      method: method || 'GET', credentials: 'same-origin', cache: 'no-store',
      headers: method === 'POST' ? { 'X-Cranibox-Desktop': '1' } : {},
      signal: controller ? controller.signal : undefined
    }).then(function (response) {
      if (timer) clearTimeout(timer);
      if (!response.ok && response.status !== 409) throw new Error('HTTP ' + response.status);
      return response.json();
    });
  }

  function startClient() {
    if (started || !entry) return;
    started = true;
    var module = document.createElement('script');
    module.type = 'module';
    module.crossOrigin = '';
    module.src = entry;
    document.head.appendChild(module);
  }

  function megabytes(value) {
    if (!value && value !== 0) return '';
    return value >= 1024 ? (value / 1024).toFixed(1).replace(/\.0$/, '') + ' GB' : Math.round(value) + ' MB';
  }

  var css = [
    '#cx-desktop{position:fixed;inset:0;z-index:2147483000;display:flex;align-items:center;justify-content:center;',
    'font-family:-apple-system,BlinkMacSystemFont,"PingFang SC","Microsoft YaHei","Noto Sans CJK SC",sans-serif;',
    'background:#faf9f6;color:#111827;-webkit-font-smoothing:antialiased}',
    '#cx-desktop .cx-card{width:min(440px,calc(100vw - 40px));background:#fff;border:1px solid rgba(17,24,39,.08);',
    'border-radius:20px;padding:36px 32px 28px;box-shadow:0 18px 50px rgba(17,24,39,.08);text-align:center}',
    '#cx-desktop .cx-icon{width:64px;height:64px;margin:0 auto 18px;border-radius:18px;display:flex;align-items:center;',
    'justify-content:center;background:rgba(16,185,129,.1);color:#059669}',
    '#cx-desktop .cx-icon.off{background:#f1f0ec;color:#636a77}',
    '#cx-desktop h1{font-size:22px;line-height:1.3;margin:0 0 10px;font-weight:650;letter-spacing:.01em}',
    '#cx-desktop p{margin:0 0 6px;font-size:14px;line-height:1.65;color:#374151}',
    '#cx-desktop .cx-muted{color:#636a77;font-size:13px}',
    '#cx-desktop .cx-facts{display:flex;gap:10px;justify-content:center;margin:18px 0 22px;flex-wrap:wrap}',
    '#cx-desktop .cx-fact{background:#f4f3ef;border-radius:12px;padding:8px 12px;font-size:13px;color:#374151}',
    '#cx-desktop .cx-fact b{font-weight:650;color:#111827}',
    '#cx-desktop button{appearance:none;border:0;border-radius:12px;font:inherit;font-size:15px;font-weight:600;',
    'padding:12px 22px;min-width:180px;cursor:pointer;display:inline-flex;align-items:center;justify-content:center;gap:8px}',
    '#cx-desktop .cx-primary{background:#10b981;color:#fff}#cx-desktop .cx-primary:hover{background:#059669}',
    '#cx-desktop .cx-secondary{background:#f1f0ec;color:#111827;min-width:120px}',
    '#cx-desktop .cx-danger{background:#dc2626;color:#fff;min-width:120px}',
    '#cx-desktop .cx-row{display:flex;gap:10px;justify-content:center;flex-wrap:wrap}',
    '#cx-desktop button[disabled]{opacity:.7;cursor:default}',
    '#cx-desktop .cx-spin{width:16px;height:16px;border-radius:50%;border:2px solid rgba(255,255,255,.45);',
    'border-top-color:#fff;animation:cx-spin .8s linear infinite}',
    '#cx-desktop .cx-spin.dark{border-color:rgba(17,24,39,.15);border-top-color:#059669;width:28px;height:28px;border-width:3px}',
    '#cx-desktop .cx-error{color:#dc2626;font-size:13px;margin-top:12px}',
    '@keyframes cx-spin{to{transform:rotate(360deg)}}',
    '#cx-desktop.cx-dialog{background:rgba(17,24,39,.45);backdrop-filter:blur(2px)}',
    // A small tab on the right edge: clear of the desktop's own panels and docks.
    '#cx-power{position:fixed;right:0;top:50%;transform:translateY(-50%);z-index:2147482000;display:flex;align-items:center;gap:6px;',
    'height:32px;padding:0 10px 0 9px;border-radius:16px 0 0 16px;border:1px solid rgba(255,255,255,.18);border-right:0;',
    'background:rgba(24,24,27,.55);color:#fff;font:600 12px -apple-system,BlinkMacSystemFont,"PingFang SC","Microsoft YaHei",sans-serif;',
    'cursor:pointer;opacity:.45;transition:opacity .2s,background .2s;backdrop-filter:blur(6px)}',
    '#cx-power span{max-width:0;overflow:hidden;white-space:nowrap;transition:max-width .2s}',
    '#cx-power:hover,#cx-power:focus-visible{opacity:1;background:rgba(24,24,27,.85)}',
    '#cx-power:hover span,#cx-power:focus-visible span{max-width:80px}',
    '@media (prefers-color-scheme:dark){#cx-desktop{background:#111113;color:#f4f4f5}',
    '#cx-desktop .cx-card{background:#1c1c1f;border-color:rgba(255,255,255,.08);box-shadow:none}',
    '#cx-desktop p{color:#d4d4d8}#cx-desktop .cx-muted{color:#a1a1aa}#cx-desktop .cx-fact{background:#27272a;color:#d4d4d8}',
    '#cx-desktop .cx-fact b{color:#fff}#cx-desktop .cx-icon.off{background:#27272a;color:#a1a1aa}',
    '#cx-desktop .cx-secondary{background:#27272a;color:#f4f4f5}#cx-desktop.cx-dialog{background:rgba(0,0,0,.55)}}'
  ].join('');

  var ICON_OFF = '<svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 17H4a2 2 0 0 1-2-2V5c0-.6.3-1.1.7-1.5"/><path d="M22 15V5a2 2 0 0 0-2-2H9"/><path d="M8 21h8"/><path d="M12 17v4"/><path d="m2 2 20 20"/></svg>';
  var ICON_ON = '<svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/></svg>';
  var ICON_POWER = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true"><path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.8 0"/></svg>';

  function ensureStyle() {
    if (document.getElementById('cx-desktop-style')) return;
    var style = document.createElement('style');
    style.id = 'cx-desktop-style';
    style.textContent = css;
    document.head.appendChild(style);
  }

  function overlay(dialog) {
    ensureStyle();
    if (!root) {
      root = document.createElement('div');
      root.id = 'cx-desktop';
      root.setAttribute('role', 'dialog');
      root.setAttribute('aria-modal', 'true');
      card = document.createElement('div');
      card.className = 'cx-card';
      root.appendChild(card);
      document.body.appendChild(root);
    }
    root.className = dialog ? 'cx-dialog' : '';
    return card;
  }

  function closeOverlay() {
    if (root) { root.remove(); root = null; card = null; }
  }

  function showOff(status, error) {
    document.title = '桌面已关闭';
    var c = overlay(false);
    var facts = '';
    if (status && status.container_memory_mb) facts += '<span class="cx-fact">当前内存占用 <b>' + megabytes(status.container_memory_mb) + '</b></span>';
    if (status && status.memory_hint_mb) facts += '<span class="cx-fact">开启后约多用 <b>' + megabytes(status.memory_hint_mb) + '</b></span>';
    c.innerHTML = '<div class="cx-icon off">' + ICON_OFF + '</div><h1>桌面已关闭</h1>' +
      '<p>为节省内存，图形桌面和桌面浏览器暂时没有运行。</p>' +
      '<p class="cx-muted">Agent、终端和其他服务不受影响，仍在正常工作。</p>' +
      (facts ? '<div class="cx-facts">' + facts + '</div>' : '<div style="height:20px"></div>') +
      '<button class="cx-primary" id="cx-on">' + ICON_POWER + '开启桌面</button>' +
      (error ? '<div class="cx-error">' + error + '</div>' : '<p class="cx-muted" style="margin-top:12px">开启通常需要 10–20 秒</p>');
    c.querySelector('#cx-on').addEventListener('click', turnOn);
    poll(function (s) { if (s.state === 'on') location.reload(); }, 5000);
  }

  function showBusy(title, detail) {
    var c = overlay(false);
    c.innerHTML = '<div class="cx-icon">' + '<span class="cx-spin dark"></span>' + '</div><h1>' + title + '</h1>' +
      '<p class="cx-muted">' + detail + '</p>';
  }

  function turnOn() {
    document.title = '正在开启桌面…';
    showBusy('正在开启桌面…', '马上就好，桌面准备好后会自动打开。');
    request('on', 'POST').then(function () {
      var deadline = Date.now() + 120000;
      poll(function (s) {
        if (s.phase === 'running') { location.reload(); return true; }
        if (s.error || Date.now() > deadline) { showOff(s, '没能开启桌面' + (s.error ? '：' + s.error : '，请稍后重试。')); return true; }
      }, 1500);
    }).catch(function () { showOff(null, '没能连接到桌面服务，请稍后重试。'); });
  }

  function confirmOff() {
    var c = overlay(true);
    c.innerHTML = '<div class="cx-icon off">' + ICON_OFF + '</div><h1>关闭桌面？</h1>' +
      '<p>桌面里打开的窗口和桌面浏览器会被关闭，未保存的内容会丢失。</p>' +
      '<p class="cx-muted">Agent、终端和其他服务会继续运行。随时可以在这个页面重新开启。</p>' +
      '<div style="height:20px"></div><div class="cx-row"><button class="cx-secondary" id="cx-cancel">取消</button>' +
      '<button class="cx-danger" id="cx-off">关闭桌面</button></div>';
    c.querySelector('#cx-cancel').addEventListener('click', closeOverlay);
    c.querySelector('#cx-off').addEventListener('click', turnOff);
    root.addEventListener('click', function (event) { if (event.target === root) closeOverlay(); });
  }

  function turnOff() {
    showBusy('正在关闭桌面…', '正在停止桌面程序并释放内存。');
    root.className = '';
    request('off', 'POST').then(function () {
      poll(function (s) { if (s.phase === 'stopped') { location.reload(); return true; } }, 1000);
    }).catch(function () { location.reload(); });
  }

  function poll(handler, interval) {
    clearTimeout(pollTimer);
    pollTimer = setTimeout(function tick() {
      request('status').then(function (s) {
        if (!handler(s)) pollTimer = setTimeout(tick, interval);
      }).catch(function () { pollTimer = setTimeout(tick, interval); });
    }, interval);
  }

  function addPowerButton() {
    ensureStyle();
    var button = document.createElement('button');
    button.id = 'cx-power';
    button.type = 'button';
    button.title = '关闭桌面以节省内存';
    button.innerHTML = ICON_POWER + '<span>关闭桌面</span>';
    button.addEventListener('click', confirmOff);
    document.body.appendChild(button);
    // Someone else switched the desktop off (CLI, another tab, the Control Center).
    poll(function (s) { if (s.state === 'off' && !root) { location.reload(); return true; } }, 10000);
  }

  function boot() {
    request('status').then(function (s) {
      if (s.state === 'off' && s.phase !== 'starting') return showOff(s);
      if (s.phase === 'starting') {
        showBusy('正在开启桌面…', '马上就好，桌面准备好后会自动打开。');
        return poll(function (n) { if (n.phase === 'running' || n.state === 'off') { location.reload(); return true; } }, 1500);
      }
      startClient();
      addPowerButton();
    }).catch(function () { startClient(); });   // no switch service: plain Selkies
  }

  if (document.body) boot(); else document.addEventListener('DOMContentLoaded', boot);
})();

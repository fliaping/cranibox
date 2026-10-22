// Evaluated before any module reads window.workspaceWeb: one language for every page.
const fallback = typeof navigator !== 'undefined' && navigator.language.toLowerCase().startsWith('zh') ? 'zh-cn' : 'en';
const config = window.workspaceWeb || (window.workspaceWeb = { csrf: '', language: fallback, cursor: 0, workspaceUrl: '', desktopUrl: '', edition: 'node' });
try {
  const preferred = localStorage.getItem('cranibox.language');
  if (preferred === 'en' || preferred === 'zh-cn') config.language = preferred;
} catch { /* Browser storage is optional. */ }
document.documentElement.lang = config.language;
export {};

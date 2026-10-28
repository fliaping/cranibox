import './boot';
import { createRoot } from 'react-dom/client';
import { HashRouter } from 'react-router-dom';
import './theme.css';
import './style.css';
import './terminal/terminal.css';
import './terminal/window.css';
import { Shell } from './app/App';
import { AppProvider } from './app/store';
import { inShell, shellCall } from './app/shell';
import { httpApi } from './personal/api';
import { TerminalStandalone } from './terminal/TerminalStandalone';
import { isTerminalWindow, TerminalWindowProvider } from './terminal/TerminalWindowContext';
import { WebTerminalEnvironment } from './terminal/WebTerminalEnvironment';
import { WorkspaceProvider } from './workspace';
import { ConfirmHost } from './confirm';

const api = httpApi(window.workspaceWeb.csrf);
// Keep bookmarks from the first control center (#overview) working.
const legacy = location.hash.slice(1);
if (/^[a-z]+$/.test(legacy)) history.replaceState(null, '', '#/' + legacy);
function Root() {
  // Language is a per-browser preference applied before any module reads the config.
  return <AppProvider api={api} language={window.workspaceWeb.language} onLocale={() => location.reload()} openExternal={inShell() ? url => shellCall('open_external', { url }) : undefined}>{isTerminalWindow() ? <TerminalStandalone /> : <Shell />}<ConfirmHost /></AppProvider>;
}
createRoot(document.getElementById('root')!).render(<HashRouter><TerminalWindowProvider><WorkspaceProvider><WebTerminalEnvironment><Root /></WebTerminalEnvironment></WorkspaceProvider></TerminalWindowProvider></HashRouter>);

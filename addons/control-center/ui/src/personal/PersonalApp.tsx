import type { ReactNode } from 'react';
import '../boot';
import { HashRouter } from 'react-router-dom';
import '../theme.css';
import '../style.css';
import { Shell } from '../app/App';
import { AppProvider } from '../app/store';
import { WebTerminalEnvironment } from '../terminal/WebTerminalEnvironment';
import { WorkspaceProvider } from '../workspace';
import type { Api } from './api';

/** Compatibility entry for shells that still embed the UI directly; new shells load the HTTP UI. */
export function PersonalApp({ api, language = 'en', openExternal, chatUrl }: { api: Api; language?: string; advanced?: ReactNode; openExternal?: (url: string) => Promise<unknown>; onNavigate?: (path: string) => void; chatUrl?: string }) {
  return <HashRouter><WorkspaceProvider><WebTerminalEnvironment><AppProvider api={api} language={language} openExternal={openExternal}><Shell chatUrl={chatUrl} /></AppProvider></WebTerminalEnvironment></WorkspaceProvider></HashRouter>;
}

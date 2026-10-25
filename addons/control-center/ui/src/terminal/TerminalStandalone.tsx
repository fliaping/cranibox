import { AccessSurfaces } from "../access/Access";
import { useEffect } from 'react';
import { TerminalWindow } from './TerminalWindow';
import { useTerminalWindow } from './TerminalWindowContext';

/** Launch requests arrive as `#/launch?node=…&session=…&create=1&n=…` from the desktop app. */
export function TerminalStandalone() {
  const { openTerminal } = useTerminalWindow();
  useEffect(() => {
    const handle = () => {
      const [path, query = ''] = location.hash.slice(1).split('?');
      if (path !== '/launch') return;
      const params = new URLSearchParams(query);
      openTerminal({ node: params.get('node') || '', session: params.get('session') || undefined, command: params.get('command') || undefined, cwd: params.get('cwd') || undefined, create: params.get('create') === '1' });
    };
    handle();
    window.addEventListener('hashchange', handle);
    return () => window.removeEventListener('hashchange', handle);
  }, [openTerminal]);
  useEffect(() => { document.title = 'Terminal · Cranibox'; }, []);
  return <TerminalWindow standalone banner={<AccessSurfaces />} />;
}

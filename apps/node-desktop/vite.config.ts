import { defineConfig } from 'vite';
import { readFileSync } from 'node:fs';

const notice = ['herdr-web-ui.txt', 'novnc.txt'].map(name => readFileSync(new URL('../../docs/licenses/' + name, import.meta.url), 'utf8')).join('\n\n');

export default defineConfig({
  build: { rolldownOptions: { output: { banner: '/*!\n' + notice + '\n*/' } } },
  resolve: { dedupe: ['react', 'react-dom', 'lucide-react', '@xterm/xterm', '@xterm/addon-fit'] },
  server: { fs: { allow: ['.', '../../addons/control-center/ui/src', '../../addons/control-center/ui/node_modules'] } },
});

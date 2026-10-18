import { defineConfig } from 'vite';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const notice = ['herdr-web-ui.txt', 'novnc.txt'].map(name => readFileSync(new URL('../../docs/licenses/' + name, import.meta.url), 'utf8')).join('\n\n');

export default defineConfig({
  build: { rolldownOptions: { output: { banner: '/*!\n' + notice + '\n*/' } } },
  resolve: {
    dedupe: ['react', 'react-dom', 'lucide-react', '@xterm/xterm', '@xterm/addon-fit', '@novnc/novnc'],
    alias: { '@cranibox/novnc-keyboard': fileURLToPath(new URL('./node_modules/@novnc/novnc/core/input/keyboard.js', import.meta.url)) },
  },
  server: { fs: { allow: ['.', '../../addons/control-center/ui/src', '../../addons/control-center/ui/node_modules'] } },
});

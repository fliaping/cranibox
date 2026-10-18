import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  define: { 'process.env.NODE_ENV': JSON.stringify('production') },
  resolve: {
    conditions: ['production', 'browser'],
    // noVNC 1.7 only exports RFB. Reuse its pinned keyboard mapping in both viewers.
    alias: { '@cranibox/novnc-keyboard': fileURLToPath(new URL('./node_modules/@novnc/novnc/core/input/keyboard.js', import.meta.url)) },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    lib: { entry: 'src/main.tsx', formats: ['es'], fileName: () => 'web.js', cssFileName: 'web' },
    sourcemap: false,
    target: 'es2022',
  },
});

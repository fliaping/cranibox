import { defineConfig } from 'vite';

export default defineConfig({
  define: { 'process.env.NODE_ENV': JSON.stringify('production') },
  resolve: { conditions: ['production', 'browser'] },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    lib: { entry: 'src/main.tsx', formats: ['es'], fileName: () => 'web.js', cssFileName: 'web' },
    sourcemap: false,
    target: 'es2022',
  },
});

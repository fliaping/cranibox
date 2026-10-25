import { defineConfig } from '@playwright/test';

// Browser-only terminal regressions: no Rust server, paired nodes or credentials.
export default defineConfig({
  testDir: './tests',
  testMatch: 'terminal.spec.ts',
  workers: 1,
  timeout: 30000,
  reporter: 'list',
  use: { baseURL: 'http://127.0.0.1:19084', viewport: { width: 1280, height: 900 }, trace: 'retain-on-failure' },
  webServer: { command: 'node tests/terminal-preview.mjs', url: 'http://127.0.0.1:19084/healthz', reuseExistingServer: false },
});

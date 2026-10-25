import { defineConfig } from '@playwright/test';

// Browser-only access controls: all API responses are supplied by each test.
export default defineConfig({
  testDir: './tests',
  testMatch: 'access.spec.ts',
  outputDir: './test-results-access',
  workers: 1,
  timeout: 30000,
  reporter: 'list',
  use: { baseURL: 'http://127.0.0.1:19084', viewport: { width: 1280, height: 900 }, trace: 'retain-on-failure' },
  webServer: { command: 'node tests/terminal-preview.mjs', url: 'http://127.0.0.1:19084/healthz', reuseExistingServer: false },
});

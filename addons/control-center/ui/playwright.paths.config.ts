import { defineConfig } from '@playwright/test';

// Run against checked-in production assets and the same path fixture as MockAppApi.
export default defineConfig({
  testDir: './tests',
  testMatch: 'network-paths.spec.ts',
  workers: 1,
  timeout: 30_000,
  reporter: 'list',
  use: { baseURL: 'http://127.0.0.1:19084', viewport: { width: 1440, height: 1000 }, trace: 'retain-on-failure' },
  webServer: { command: 'node tests/paths-preview.mjs', url: 'http://127.0.0.1:19084/healthz', reuseExistingServer: false },
});

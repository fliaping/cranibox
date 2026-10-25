import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  testMatch: '**/*.spec.ts',
  // These suites have their own browser-only previews and configurations.
  testIgnore: ['**/terminal.spec.ts', '**/network-paths.spec.ts', '**/access.spec.ts'],
  fullyParallel: false,
  workers: 1,
  timeout: 30000,
  reporter: 'list',
  use: { baseURL: 'http://127.0.0.1:19080', viewport: { width: 1440, height: 1000 }, trace: 'retain-on-failure' },
  webServer: [
    { command: 'node tests/preview.mjs', url: 'http://127.0.0.1:19080/healthz', reuseExistingServer: false },
    { command: 'node tests/preview.mjs', env: { UI_PREVIEW_PORT: '19082', UI_PREVIEW_LANGUAGE: 'zh-cn' }, url: 'http://127.0.0.1:19082/healthz', reuseExistingServer: false },
  ],
});

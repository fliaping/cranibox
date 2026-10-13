import { defineConfig } from '@playwright/test';
export default defineConfig({ testDir: './tests', fullyParallel: true, use: { baseURL: 'http://127.0.0.1:5174', headless: true }, webServer: { command: 'npm run dev -- --host 127.0.0.1', url: 'http://127.0.0.1:5174', reuseExistingServer: false }, reporter: 'list' });

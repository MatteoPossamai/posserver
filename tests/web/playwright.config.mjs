import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: 'ui.spec.mjs',
  workers: 1,
  use: { viewport: { width: 390, height: 844 } },
  reporter: 'list',
});

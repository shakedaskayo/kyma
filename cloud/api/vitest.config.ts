import { defineConfig } from 'vitest/config';
import path from 'node:path';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    setupFiles: ['./src/test-setup.ts'],
    testTimeout: 20000,
    env: {
      // Provide a test-safe SESSION_SECRET so env.ts validation passes.
      // Real secrets come from ../.env.local when running the dev server.
      SESSION_SECRET: process.env.SESSION_SECRET ??
        'test-session-secret-32-chars-minimum!!',
    },
  },
});

import { defineConfig } from 'vitest/config';
import path from 'node:path';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    setupFiles: ['./src/test-setup.ts'],
    testTimeout: 20000,
    // Integration tests share a single Postgres database (`kyma_cloud_test`)
    // and the freshDb() helper drops+recreates it. Running test files in
    // parallel worker processes races on CREATE DATABASE. Force single-fork
    // execution so freshDb() calls serialize.
    pool: 'forks',
    poolOptions: {
      forks: { singleFork: true },
    },
    env: {
      // Provide a test-safe SESSION_SECRET so env.ts validation passes.
      // Real secrets come from ../.env.local when running the dev server.
      SESSION_SECRET: process.env.SESSION_SECRET ??
        'test-session-secret-32-chars-minimum!!',
    },
  },
});

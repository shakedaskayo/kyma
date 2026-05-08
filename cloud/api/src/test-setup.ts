// Globals for Vitest. Each test file should call `await freshDb()` in its
// beforeAll to get an isolated logical DB.
import { execSync } from 'node:child_process';
import { closeDb } from './db/client.js';

export async function freshDb(): Promise<void> {
  const url = process.env.DRIZZLE_DATABASE_URL ??
    'postgres://kyma:kyma_dev@localhost:5434/kyma_cloud_test';
  process.env.DRIZZLE_DATABASE_URL = url;
  const dbName = url.split('/').pop()!;
  const adminUrl = url.replace(/\/[^/]+$/, '/postgres');
  execSync(
    `psql "${adminUrl}" -c "DROP DATABASE IF EXISTS ${dbName} WITH (FORCE);"`,
    { stdio: 'pipe' },
  );
  execSync(`psql "${adminUrl}" -c "CREATE DATABASE ${dbName};"`, { stdio: 'pipe' });
  execSync('pnpm --filter @kyma/cloud-api run db:migrate', {
    stdio: 'inherit',
    env: {
      ...process.env,
      DRIZZLE_DATABASE_URL: url,
      // migrate.ts validates SESSION_SECRET via env.ts; supply a test-safe value
      SESSION_SECRET: process.env.SESSION_SECRET ?? 'test-session-secret-32-chars-minimum!!',
    },
  });
  await closeDb();
}

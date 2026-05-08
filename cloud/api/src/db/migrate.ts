import 'dotenv/config';
import { migrate } from 'drizzle-orm/node-postgres/migrator';
import { getDb, closeDb } from './client.js';

async function main() {
  console.log('[cloud] running migrations...');
  await migrate(getDb(), { migrationsFolder: './src/db/migrations' });
  console.log('[cloud] migrations complete.');
  await closeDb();
}

main().catch((err) => {
  console.error('[cloud] migration failed:', err);
  process.exit(1);
});

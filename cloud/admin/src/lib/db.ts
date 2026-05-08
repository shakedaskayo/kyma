import { drizzle } from 'drizzle-orm/node-postgres';
import pg from 'pg';

let _pool: pg.Pool | null = null;
let _db: ReturnType<typeof drizzle> | null = null;

export function getDb() {
  if (_db) return _db;
  _pool = new pg.Pool({
    connectionString: process.env.DRIZZLE_DATABASE_URL ?? 'postgres://kyma:kyma_dev@localhost:5434/kyma_cloud',
    max: 10,
  });
  _db = drizzle(_pool);
  return _db;
}

export function getPool() {
  if (!_pool) getDb();
  return _pool!;
}

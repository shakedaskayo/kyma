import 'server-only';
import pg from 'pg';

let _pool: pg.Pool | null = null;
function pool() {
  if (_pool) return _pool;
  _pool = new pg.Pool({ connectionString: process.env.DRIZZLE_DATABASE_URL });
  return _pool;
}

export async function listWorkspaces() {
  const { rows } = await pool().query(`
    SELECT
      w.id, w.slug, w.name, w.plan, w.kind, w.plan_active,
      w.stripe_customer_id, w.created_at,
      u.email AS owner_email
    FROM workspaces w
    JOIN users u ON u.id = w.owner_user_id
    ORDER BY w.created_at DESC
    LIMIT 200
  `);
  return rows;
}

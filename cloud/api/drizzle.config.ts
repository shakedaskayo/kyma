import 'dotenv/config';
import { defineConfig } from 'drizzle-kit';

export default defineConfig({
  schema: './src/db/schema.ts',
  out: './src/db/migrations',
  dialect: 'postgresql',
  dbCredentials: {
    url:
      process.env.DRIZZLE_DATABASE_URL ||
      'postgres://kyma:kyma_dev@localhost:5434/kyma_cloud',
  },
});

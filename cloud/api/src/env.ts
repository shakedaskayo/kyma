import 'dotenv/config';
import { z } from 'zod';

const envSchema = z.object({
  PORT: z.coerce.number().default(3003),
  NODE_ENV: z.enum(['development', 'production', 'test']).default('development'),

  DRIZZLE_DATABASE_URL: z.string().default(
    'postgres://kyma:kyma_dev@localhost:5434/kyma_cloud',
  ),

  GITHUB_CLIENT_ID: z.string().default(''),
  GITHUB_CLIENT_SECRET: z.string().default(''),

  RESEND_API_KEY: z.string().default(''),
  RESEND_FROM_EMAIL: z.string().default('noreply@kyma.dev'),

  STRIPE_SECRET_KEY: z.string().optional(),
  STRIPE_WEBHOOK_SIGNING_SECRET: z.string().optional(),
  STRIPE_PRICE_PRO: z.string().optional(),
  STRIPE_PRICE_TEAM: z.string().optional(),

  SESSION_SECRET: z.string().min(32),

  CLOUD_BASE_URL: z.string().url().default('http://localhost:3001'),
  ADMIN_BASE_URL: z.string().url().default('http://localhost:3002'),

  KYMA_ENGINE_BASE_URL: z.string().url().default('http://localhost:8080'),

  KYMA_ADMIN_GITHUB_IDS: z.string().default(''),
});

export type Env = z.infer<typeof envSchema>;

let _env: Env | null = null;
export function getEnv(): Env {
  if (_env) return _env;
  const parsed = envSchema.safeParse(process.env);
  if (!parsed.success) {
    console.error('Invalid env:');
    for (const issue of parsed.error.issues) {
      console.error(`  ${issue.path.join('.')}: ${issue.message}`);
    }
    process.exit(1);
  }
  _env = parsed.data;
  return _env;
}

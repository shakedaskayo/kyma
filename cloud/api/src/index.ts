import { serve } from '@hono/node-server';
import { Hono } from 'hono';
import { cors } from 'hono/cors';
import { logger } from 'hono/logger';
import { getEnv } from './env.js';
import { AppError } from './lib/errors.js';
import { authRoutes } from './routes/auth.js';
import { workspaceRoutes } from './routes/workspaces.js';
import { billingRoutes } from './routes/billing.js';
import { stripeWebhookRoutes } from './routes/webhooks.js';

export function buildApp() {
  const app = new Hono();
  app.use('*', logger());
  app.use('*', cors({
    origin: (o) => o || '*',
    credentials: true,
    allowMethods: ['GET', 'POST', 'PATCH', 'DELETE', 'OPTIONS'],
    allowHeaders: ['Content-Type', 'Authorization', 'Cookie'],
  }));

  app.get('/health', (c) => c.json({ status: 'ok', service: 'kyma-cloud-api' }));

  // Public webhook — must be mounted BEFORE auth-gated routes. Reads raw body
  // for Stripe signature verification.
  app.route('/api/webhooks', stripeWebhookRoutes);

  app.route('/api/auth', authRoutes);
  app.route('/api/workspaces', workspaceRoutes);
  app.route('/api/billing', billingRoutes);

  app.onError((err, c) => {
    if (err instanceof AppError) {
      return c.json({ error: { code: err.code, message: err.message } }, err.statusCode as any);
    }
    console.error('[cloud] unhandled:', err);
    return c.json({ error: { code: 'INTERNAL_ERROR', message: 'Internal server error' } }, 500);
  });

  app.notFound((c) => c.json({ error: { code: 'NOT_FOUND', message: 'Route not found' } }, 404));

  return app;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const env = getEnv();
  const app = buildApp();
  console.log(`[cloud] listening on :${env.PORT}`);
  serve({ fetch: app.fetch, port: env.PORT });
}

import { describe, it, expect, beforeAll, afterAll, vi } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';

vi.mock('./stripe.service.js', async (importOriginal) => {
  const actual = await importOriginal() as typeof import('./stripe.service.js');
  return {
    ...actual,
    isStripeConfigured: () => true,
    getStripe: () => ({
      subscriptions: { list: async (_args: any) => ({ data: [] }) },
    }) as any,
  };
});

import { reconcileOnce } from './billing-reconciler.js';

beforeAll(async () => { process.env.SESSION_SECRET = 'a'.repeat(48); await freshDb(); });
afterAll(async () => { await closeDb(); });

describe('billing-reconciler', () => {
  it('scans workspaces with stripeCustomerId; downgrades when no active sub', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
    await db.insert(schema.workspaces).values({
      slug: 'a', name: 'A', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/x/mcp/v1',
      stripeCustomerId: 'cus_123', stripeSubscriptionId: 'sub_existing', plan: 'pro',
    });
    const r = await reconcileOnce();
    expect(r.scanned).toBe(1);
    expect(r.updated).toBe(1);
    const [ws] = await db.select().from(schema.workspaces);
    expect(ws.plan).toBe('free');
    expect(ws.stripeSubscriptionId).toBeNull();
  });
});

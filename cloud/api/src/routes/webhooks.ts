import { Hono } from 'hono';
import type Stripe from 'stripe';
import { eq } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getEnv } from '../env.js';
import {
  getStripe, isStripeConfigured, applyWorkspaceSubscription,
  downgradeWorkspaceToFree, findWorkspaceByCustomerId,
} from '../services/stripe.service.js';

export const stripeWebhookRoutes = new Hono();

stripeWebhookRoutes.post('/stripe', async (c) => {
  const env = getEnv();
  if (!env.STRIPE_WEBHOOK_SIGNING_SECRET) {
    return c.json({ error: { code: 'WEBHOOK_DISABLED' } }, 503);
  }
  if (!isStripeConfigured()) {
    return c.json({ error: { code: 'BILLING_UNAVAILABLE' } }, 503);
  }
  const sig = c.req.header('stripe-signature');
  if (!sig) return c.json({ error: { code: 'NO_SIGNATURE' } }, 400);

  const raw = await c.req.raw.text();
  let event: Stripe.Event;
  try {
    event = getStripe().webhooks.constructEvent(raw, sig, env.STRIPE_WEBHOOK_SIGNING_SECRET);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error('[stripe.webhook] sig verify failed:', msg);
    return c.json({ error: { code: 'INVALID_SIGNATURE', message: msg } }, 400);
  }

  const db = getDb();
  const inserted = await db.insert(schema.billingEvents).values({
    stripeEventId: event.id,
    eventType: event.type,
    payload: event as unknown as Record<string, unknown>,
    processed: false,
  }).onConflictDoNothing({ target: schema.billingEvents.stripeEventId })
    .returning({ id: schema.billingEvents.id });

  if (inserted.length === 0) {
    return c.json({ received: true, duplicate: true });
  }

  try {
    await dispatch(event);
    await db.update(schema.billingEvents)
      .set({ processed: true })
      .where(eq(schema.billingEvents.stripeEventId, event.id));
  } catch (err) {
    console.error(`[stripe.webhook] handler ${event.type} failed:`, err);
    // Intentionally still 200 — the row stays processed=false in billing_events
    // and the hourly reconciler picks it up. Returning 5xx triggers Stripe's
    // 3-day retry storm.
  }
  return c.json({ received: true });
});

async function dispatch(event: Stripe.Event): Promise<void> {
  const db = getDb();
  const tagWs = (eventId: string, workspaceId: string) =>
    db.update(schema.billingEvents).set({ workspaceId }).where(eq(schema.billingEvents.stripeEventId, eventId));

  switch (event.type) {
    case 'customer.subscription.created':
    case 'customer.subscription.updated': {
      const sub = event.data.object as Stripe.Subscription;
      const customerId = typeof sub.customer === 'string' ? sub.customer : sub.customer.id;
      const ws = await findWorkspaceByCustomerId(customerId);
      if (!ws) { console.warn(`[stripe.webhook] ${event.type} for unknown customer ${customerId}`); return; }
      const metaWsId = sub.metadata?.workspace_id;
      if (metaWsId && metaWsId !== ws.id) {
        console.error(`[stripe.webhook] metadata.workspace_id=${metaWsId} mismatches customer's workspace=${ws.id}; refusing`);
        return;
      }
      await applyWorkspaceSubscription(ws.id, sub);
      await tagWs(event.id, ws.id);
      return;
    }
    case 'customer.subscription.deleted': {
      const sub = event.data.object as Stripe.Subscription;
      const customerId = typeof sub.customer === 'string' ? sub.customer : sub.customer.id;
      const ws = await findWorkspaceByCustomerId(customerId);
      if (!ws) return;
      await downgradeWorkspaceToFree(ws.id);
      await tagWs(event.id, ws.id);
      return;
    }
    case 'invoice.payment_failed': {
      const inv = event.data.object as Stripe.Invoice;
      const customerId = typeof inv.customer === 'string' ? inv.customer : inv.customer?.id;
      if (!customerId) return;
      const ws = await findWorkspaceByCustomerId(customerId);
      if (!ws) return;
      await db.update(schema.workspaces).set({
        planActive: false, dunningState: 'failed', updatedAt: new Date(),
      }).where(eq(schema.workspaces.id, ws.id));
      await tagWs(event.id, ws.id);
      return;
    }
    case 'invoice.paid': {
      const inv = event.data.object as Stripe.Invoice;
      const customerId = typeof inv.customer === 'string' ? inv.customer : inv.customer?.id;
      if (!customerId) return;
      const ws = await findWorkspaceByCustomerId(customerId);
      if (!ws) return;
      await db.update(schema.workspaces).set({
        planActive: true, dunningState: 'paid', updatedAt: new Date(),
      }).where(eq(schema.workspaces.id, ws.id));
      await tagWs(event.id, ws.id);
      return;
    }
  }
}

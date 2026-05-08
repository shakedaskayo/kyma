import { isNotNull } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getStripe, isStripeConfigured, applyWorkspaceSubscription, downgradeWorkspaceToFree } from './stripe.service.js';

export async function reconcileOnce(): Promise<{ scanned: number; updated: number }> {
  if (!isStripeConfigured()) return { scanned: 0, updated: 0 };
  const db = getDb();
  const stripe = getStripe();
  const wss = await db.select({
    id: schema.workspaces.id,
    customerId: schema.workspaces.stripeCustomerId,
    subscriptionId: schema.workspaces.stripeSubscriptionId,
  }).from(schema.workspaces).where(isNotNull(schema.workspaces.stripeCustomerId));

  let updated = 0;
  for (const ws of wss) {
    if (!ws.customerId) continue;
    try {
      const subs = await stripe.subscriptions.list({ customer: ws.customerId, status: 'all', limit: 5 });
      const active = subs.data.find(s => s.status === 'active' || s.status === 'trialing');
      if (active) {
        await applyWorkspaceSubscription(ws.id, active);
        updated += 1;
      } else if (ws.subscriptionId) {
        await downgradeWorkspaceToFree(ws.id);
        updated += 1;
      }
    } catch (err) {
      console.error(`[billing.reconcile] ws=${ws.id} failed:`, err);
    }
  }
  return { scanned: wss.length, updated };
}

export function startReconciler(intervalMs = 60 * 60 * 1000): () => void {
  const tick = () => {
    reconcileOnce()
      .then((r) => console.log(`[billing.reconcile] scanned=${r.scanned} updated=${r.updated}`))
      .catch((err) => console.error('[billing.reconcile] failed:', err));
  };
  tick();
  const handle = setInterval(tick, intervalMs);
  return () => clearInterval(handle);
}

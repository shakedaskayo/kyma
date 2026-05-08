import Stripe from 'stripe';
import { eq } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getEnv } from '../env.js';
import type { PlanId } from '@kyma/shared';
import { PLANS } from '@kyma/shared';

// Pinned per Slice 2 spec — bumping must be a deliberate code change. The
// stripe@17 SDK's LatestApiVersion is currently 2025-02-24.acacia, but Slice
// 2 pins to the version we tested against.
export const STRIPE_API_VERSION = '2024-11-20.acacia' as const;

let _client: Stripe | null = null;
export function getStripe(): Stripe {
  if (_client) return _client;
  const key = getEnv().STRIPE_SECRET_KEY;
  if (!key) throw new Error('STRIPE_SECRET_KEY not set');
  _client = new Stripe(key, { apiVersion: STRIPE_API_VERSION as any });
  return _client;
}

export function isStripeConfigured(): boolean {
  return Boolean(getEnv().STRIPE_SECRET_KEY);
}

export function getPriceIdForPlan(plan: PlanId): string | null {
  const env = getEnv();
  if (plan === 'pro')  return env.STRIPE_PRICE_PRO  ?? null;
  if (plan === 'team') return env.STRIPE_PRICE_TEAM ?? null;
  return null;
}

export function getPlanForPriceId(priceId: string): PlanId | null {
  const env = getEnv();
  if (env.STRIPE_PRICE_PRO  && priceId === env.STRIPE_PRICE_PRO)  return 'pro';
  if (env.STRIPE_PRICE_TEAM && priceId === env.STRIPE_PRICE_TEAM) return 'team';
  return null;
}

export async function findWorkspaceByCustomerId(customerId: string) {
  const db = getDb();
  const [row] = await db.select({ id: schema.workspaces.id })
    .from(schema.workspaces)
    .where(eq(schema.workspaces.stripeCustomerId, customerId))
    .limit(1);
  return row ?? null;
}

export async function getOrCreateStripeCustomer(workspace: {
  id: string; ownerUserId: string; stripeCustomerId: string | null;
}): Promise<Stripe.Customer> {
  const stripe = getStripe();
  const db = getDb();
  if (workspace.stripeCustomerId) {
    const c = await stripe.customers.retrieve(workspace.stripeCustomerId);
    if (!('deleted' in c) || !c.deleted) return c as Stripe.Customer;
  }
  const [owner] = await db.select({ email: schema.users.email })
    .from(schema.users).where(eq(schema.users.id, workspace.ownerUserId)).limit(1);
  const customer = await stripe.customers.create({
    email: owner?.email,
    metadata: { workspace_id: workspace.id, owner_email: owner?.email ?? '' },
  });
  await db.update(schema.workspaces)
    .set({ stripeCustomerId: customer.id, updatedAt: new Date() })
    .where(eq(schema.workspaces.id, workspace.id));
  return customer;
}

/** Idempotent — webhook may invoke multiple times for the same subscription. */
export async function applyWorkspaceSubscription(workspaceId: string, sub: Stripe.Subscription): Promise<void> {
  const db = getDb();
  const planItem = sub.items.data.find((it) => it.price?.recurring?.usage_type !== 'metered');
  const priceId = planItem?.price.id ?? null;
  const plan: PlanId = priceId ? (getPlanForPriceId(priceId) ?? 'free') : 'free';

  const status = sub.status;
  const planActive = status === 'active' || status === 'trialing';
  const trialEndsAt = sub.trial_end ? new Date(sub.trial_end * 1000) : null;

  const periodEndUnix =
    (planItem as unknown as { current_period_end?: number } | undefined)?.current_period_end
    ?? (sub as unknown as { current_period_end?: number }).current_period_end
    ?? null;
  const subscriptionPeriodEnd = periodEndUnix ? new Date(periodEndUnix * 1000) : null;

  await db.update(schema.workspaces).set({
    plan,
    planActive,
    stripeSubscriptionId: sub.id,
    trialEndsAt,
    subscriptionPeriodEnd,
    dunningState: status === 'past_due' || status === 'unpaid' ? 'past_due' : 'paid',
    updatedAt: new Date(),
  }).where(eq(schema.workspaces.id, workspaceId));
}

export async function downgradeWorkspaceToFree(workspaceId: string): Promise<void> {
  const db = getDb();
  await db.update(schema.workspaces).set({
    plan: 'free', planActive: true,
    stripeSubscriptionId: null, trialEndsAt: null, subscriptionPeriodEnd: null,
    dunningState: null, updatedAt: new Date(),
  }).where(eq(schema.workspaces.id, workspaceId));
}

export { PLANS };

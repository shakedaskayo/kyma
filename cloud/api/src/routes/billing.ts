import { Hono } from 'hono';
import { z } from 'zod';
import { sessionMiddleware } from '../middleware/session.js';
import { getEnv } from '../env.js';
import { AppError, badRequest, forbidden } from '../lib/errors.js';
import type { HonoEnv } from '../lib/hono-env.js';
import * as ws from '../services/workspace.service.js';
import {
  getStripe, isStripeConfigured, getPriceIdForPlan, getOrCreateStripeCustomer,
} from '../services/stripe.service.js';

export const billingRoutes = new Hono<HonoEnv>();
billingRoutes.use('*', sessionMiddleware);

function billingDisabled() {
  return new AppError(503, 'BILLING_UNAVAILABLE', 'Stripe is not configured on this server');
}

const checkoutSchema = z.object({
  workspaceSlug: z.string(),
  plan: z.enum(['pro', 'team']),
  returnUrl: z.string().url().optional(),
});

billingRoutes.post('/checkout', async (c) => {
  if (!isStripeConfigured()) throw billingDisabled();
  const u = c.get('user');
  const body = await c.req.json();
  const parsed = checkoutSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);

  const { workspace, role } = await ws.getBySlugForUser(u.userId, parsed.data.workspaceSlug);
  if (!['owner', 'admin'].includes(role)) throw forbidden('Only owner or admin can change plan');
  const priceId = getPriceIdForPlan(parsed.data.plan);
  if (!priceId) throw badRequest(`Plan ${parsed.data.plan} not configured`, 'PLAN_UNCONFIGURED');

  const customer = await getOrCreateStripeCustomer({
    id: workspace.id,
    ownerUserId: workspace.ownerUserId,
    stripeCustomerId: workspace.stripeCustomerId,
  });

  const env = getEnv();
  const baseReturn = parsed.data.returnUrl ?? `${env.CLOUD_BASE_URL}/workspaces/${workspace.slug}/billing`;
  const cleanBase = baseReturn.split('?')[0];
  const stripe = getStripe();

  const session = await stripe.checkout.sessions.create({
    mode: 'subscription',
    customer: customer.id,
    client_reference_id: workspace.id,
    line_items: [{ price: priceId, quantity: 1 }],
    success_url: `${cleanBase}?status=success&session_id={CHECKOUT_SESSION_ID}`,
    cancel_url: `${cleanBase}?status=cancelled`,
    allow_promotion_codes: true,
    subscription_data: {
      metadata: { workspace_id: workspace.id },
      ...(workspace.stripeSubscriptionId ? {} : { trial_period_days: 14 }),
    },
  });
  return c.json({ url: session.url });
});

billingRoutes.post('/portal', async (c) => {
  if (!isStripeConfigured()) throw billingDisabled();
  const u = c.get('user');
  const body = await c.req.json();
  const parsed = z.object({
    workspaceSlug: z.string(),
    returnUrl: z.string().url().optional(),
  }).safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const { workspace, role } = await ws.getBySlugForUser(u.userId, parsed.data.workspaceSlug);
  if (!['owner', 'admin'].includes(role)) throw forbidden('Only owner or admin can open billing portal');
  if (!workspace.stripeCustomerId) throw badRequest('No customer yet — start a checkout first', 'NO_CUSTOMER');

  const env = getEnv();
  const session = await getStripe().billingPortal.sessions.create({
    customer: workspace.stripeCustomerId,
    return_url: parsed.data.returnUrl ?? `${env.CLOUD_BASE_URL}/workspaces/${workspace.slug}/billing`,
  });
  return c.json({ url: session.url });
});

billingRoutes.get('/:slug/subscription', async (c) => {
  const u = c.get('user');
  const { workspace } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  return c.json({
    plan: workspace.plan,
    planActive: workspace.planActive,
    trialEndsAt: workspace.trialEndsAt,
    currentPeriodEnd: workspace.subscriptionPeriodEnd,
    stripeCustomerId: workspace.stripeCustomerId,
    stripeSubscriptionId: workspace.stripeSubscriptionId,
    dunningState: workspace.dunningState,
  });
});

import { describe, it, expect, beforeAll } from 'vitest';
import { STRIPE_API_VERSION, getPriceIdForPlan, getPlanForPriceId, isStripeConfigured } from './stripe.service.js';

describe('stripe.service config', () => {
  beforeAll(() => {
    process.env.STRIPE_SECRET_KEY = '';
    process.env.STRIPE_PRICE_PRO = 'price_pro_x';
    process.env.STRIPE_PRICE_TEAM = 'price_team_x';
    process.env.SESSION_SECRET = 'a'.repeat(48);
  });

  it('pins API version to 2024-11-20.acacia', () => {
    expect(STRIPE_API_VERSION).toBe('2024-11-20.acacia');
  });
  it('isStripeConfigured returns false when secret missing', () => {
    expect(isStripeConfigured()).toBe(false);
  });
  it('plan ↔ price helpers round-trip', () => {
    expect(getPriceIdForPlan('pro')).toBe('price_pro_x');
    expect(getPlanForPriceId('price_pro_x')).toBe('pro');
    expect(getPlanForPriceId('unknown')).toBe(null);
  });
});

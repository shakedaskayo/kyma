export type PlanId = 'free' | 'pro' | 'team';

export interface PlanLimits {
  maxWorkspaces: number;
  ingestBytesPerMonth: number;
  mcpCallsPerDay: number;
  pricePerMonth: number;
  trialDays: number;
  features: string[];
}

export const PLANS: Record<PlanId, PlanLimits> = {
  free: {
    maxWorkspaces: 1,
    ingestBytesPerMonth: 1 * 1024 * 1024 * 1024,
    mcpCallsPerDay: 50,
    pricePerMonth: 0,
    trialDays: 0,
    features: ['1 workspace', '1 GiB / month ingest', '50 MCP calls / day', 'Community support'],
  },
  pro: {
    maxWorkspaces: 5,
    ingestBytesPerMonth: 50 * 1024 * 1024 * 1024,
    mcpCallsPerDay: 5000,
    pricePerMonth: 49,
    trialDays: 14,
    features: ['5 workspaces', '50 GiB / month ingest', '5,000 MCP calls / day', 'Email support'],
  },
  team: {
    maxWorkspaces: 20,
    ingestBytesPerMonth: 1024 * 1024 * 1024 * 1024,
    mcpCallsPerDay: -1,
    pricePerMonth: 299,
    trialDays: 14,
    features: ['20 workspaces', '1 TiB / month ingest', 'Unlimited MCP calls', 'Priority support', 'SSO (Slice 5)'],
  },
};

export function getPlanLimits(plan: PlanId): PlanLimits {
  return PLANS[plan];
}

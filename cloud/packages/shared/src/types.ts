export interface CloudUser {
  id: string;
  githubId: string | null;
  email: string;
  name: string | null;
  avatarUrl: string | null;
  createdAt: string;
}

export interface Workspace {
  id: string;
  slug: string;
  name: string;
  ownerUserId: string;
  plan: 'free' | 'pro' | 'team';
  planActive: boolean;
  kind: 'shared' | 'dedicated';
  kymaEndpoint: string;       // engine HTTP base, e.g. https://mcp.kyma.dev
  mcpEndpoint: string;        // single shared URL: <KYMA_ENGINE_BASE_URL>/mcp/v1 — bearer token differentiates workspaces
  trialEndsAt: string | null;
  subscriptionPeriodEnd: string | null;
  createdAt: string;
}

export interface WorkspaceMember {
  workspaceId: string;
  userId: string;
  role: 'owner' | 'admin' | 'member';
  joinedAt: string;
}

export interface ApiTokenSummary {
  id: string;
  name: string;
  prefix: string;             // e.g. "kyma_a1b2c3d4"
  scopes: string[];           // ['read'] | ['read','write'] | ['read','write','admin']
  createdAt: string;
  lastUsedAt: string | null;
  revokedAt: string | null;
}

export interface SessionUser {
  id: string;
  email: string;
  name: string | null;
}

import {
  pgTable, uuid, varchar, text, boolean, integer, bigint, real,
  timestamp, jsonb, uniqueIndex, index, primaryKey, customType,
} from 'drizzle-orm/pg-core';

// bytea custom type — Drizzle's native Buffer mapping.
const bytea = customType<{ data: Buffer; driverData: Buffer }>({
  dataType() { return 'bytea'; },
});

// ─── users ─────────────────────────────────────────────────────────────────
export const users = pgTable('users', {
  id: uuid('id').primaryKey().defaultRandom(),
  githubId: varchar('github_id', { length: 64 }).unique(),
  email: varchar('email', { length: 255 }).notNull().unique(),
  name: varchar('name', { length: 255 }),
  avatarUrl: text('avatar_url'),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

// ─── workspaces ────────────────────────────────────────────────────────────
export const workspaces = pgTable('workspaces', {
  id: uuid('id').primaryKey().defaultRandom(),
  slug: varchar('slug', { length: 64 }).notNull().unique(),
  name: varchar('name', { length: 255 }).notNull(),
  ownerUserId: uuid('owner_user_id').notNull().references(() => users.id),
  plan: varchar('plan', { length: 32 }).notNull().default('free'),
  planActive: boolean('plan_active').notNull().default(true),
  kind: varchar('kind', { length: 32 }).notNull().default('shared'),
  kymaEndpoint: text('kyma_endpoint').notNull(),
  mcpEndpoint: text('mcp_endpoint').notNull(),
  stripeCustomerId: varchar('stripe_customer_id', { length: 255 }).unique(),
  stripeSubscriptionId: varchar('stripe_subscription_id', { length: 255 }).unique(),
  trialEndsAt: timestamp('trial_ends_at', { withTimezone: true }),
  subscriptionPeriodEnd: timestamp('subscription_period_end', { withTimezone: true }),
  dunningState: varchar('dunning_state', { length: 16 }),
  dedicatedRailwayProjectId: varchar('dedicated_railway_project_id', { length: 255 }),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

// ─── workspace_members ─────────────────────────────────────────────────────
export const workspaceMembers = pgTable('workspace_members', {
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  userId: uuid('user_id').notNull().references(() => users.id, { onDelete: 'cascade' }),
  role: varchar('role', { length: 32 }).notNull().default('member'),
  joinedAt: timestamp('joined_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  primaryKey({ columns: [t.workspaceId, t.userId] }),
]);

// ─── api_tokens ────────────────────────────────────────────────────────────
// Shared with kyma-server's DbAuthBackend (crates/kyma-server/src/auth/db_backend.rs).
// MUST match these columns exactly: tenant_id, token_hash (bytea), scopes (text),
// subject, last_used_at, revoked_at, created_at. Extra columns (workspace_id,
// name, prefix) are cloud-side only and ignored by the engine.
export const apiTokens = pgTable('api_tokens', {
  id: uuid('id').primaryKey().defaultRandom(),
  tenantId: uuid('tenant_id').notNull(),                 // = workspace.id
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  tokenHash: bytea('token_hash').notNull(),
  scopes: text('scopes').notNull().default('read,write'),
  subject: text('subject'),
  name: varchar('name', { length: 128 }),
  prefix: varchar('prefix', { length: 32 }),
  createdByUserId: uuid('created_by_user_id').references(() => users.id, { onDelete: 'set null' }),
  lastUsedAt: timestamp('last_used_at', { withTimezone: true }),
  revokedAt: timestamp('revoked_at', { withTimezone: true }),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  uniqueIndex('api_tokens_token_hash_uniq').on(t.tokenHash),
  index('api_tokens_workspace_idx').on(t.workspaceId),
  index('api_tokens_tenant_idx').on(t.tenantId),
]);

// ─── magic_links ───────────────────────────────────────────────────────────
export const magicLinks = pgTable('magic_links', {
  id: uuid('id').primaryKey().defaultRandom(),
  email: varchar('email', { length: 255 }).notNull(),
  tokenHash: bytea('token_hash').notNull(),
  expiresAt: timestamp('expires_at', { withTimezone: true }).notNull(),
  consumedAt: timestamp('consumed_at', { withTimezone: true }),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  uniqueIndex('magic_links_token_hash_uniq').on(t.tokenHash),
  index('magic_links_email_idx').on(t.email),
]);

// ─── billing_events ────────────────────────────────────────────────────────
export const billingEvents = pgTable('billing_events', {
  id: uuid('id').primaryKey().defaultRandom(),
  stripeEventId: varchar('stripe_event_id', { length: 255 }).notNull().unique(),
  eventType: varchar('event_type', { length: 255 }).notNull(),
  workspaceId: uuid('workspace_id').references(() => workspaces.id, { onDelete: 'set null' }),
  payload: jsonb('payload').notNull(),
  processed: boolean('processed').notNull().default(false),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
});

// ─── credits ───────────────────────────────────────────────────────────────
export const credits = pgTable('credits', {
  id: uuid('id').primaryKey().defaultRandom(),
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  amount: real('amount').notNull(),
  remaining: real('remaining').notNull(),
  source: varchar('source', { length: 64 }).notNull(),
  stripePaymentId: varchar('stripe_payment_id', { length: 255 }),
  expiresAt: timestamp('expires_at', { withTimezone: true }),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  index('credits_workspace_idx').on(t.workspaceId),
]);

// ─── usage_events ──────────────────────────────────────────────────────────
// Append-only — kind in: 'compute_seconds' | 'ingest_bytes' | 'agent_calls' | 'mcp_calls'
export const usageEvents = pgTable('usage_events', {
  id: uuid('id').primaryKey().defaultRandom(),
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  kind: varchar('kind', { length: 64 }).notNull(),
  value: bigint('value', { mode: 'number' }).notNull(),
  occurredAt: timestamp('occurred_at', { withTimezone: true }).notNull(),
  recordedAt: timestamp('recorded_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  index('usage_events_workspace_occurred_idx').on(t.workspaceId, t.occurredAt),
]);

// ─── usage_daily ───────────────────────────────────────────────────────────
export const usageDaily = pgTable('usage_daily', {
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  day: timestamp('day', { withTimezone: true, mode: 'date' }).notNull(),
  kind: varchar('kind', { length: 64 }).notNull(),
  total: bigint('total', { mode: 'number' }).notNull().default(0),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  primaryKey({ columns: [t.workspaceId, t.day, t.kind] }),
]);

// ─── platform_admin_audit ──────────────────────────────────────────────────
export const platformAdminAudit = pgTable('platform_admin_audit', {
  id: uuid('id').primaryKey().defaultRandom(),
  adminUserId: uuid('admin_user_id').references(() => users.id, { onDelete: 'set null' }),
  action: varchar('action', { length: 64 }).notNull(),
  targetUserId: uuid('target_user_id').references(() => users.id, { onDelete: 'set null' }),
  targetWorkspaceId: uuid('target_workspace_id').references(() => workspaces.id, { onDelete: 'set null' }),
  payload: jsonb('payload').notNull().default({}),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  index('platform_admin_audit_created_at_idx').on(t.createdAt),
]);

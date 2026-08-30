// Shim: backwards-compatible wrappers around @pensieve-ai/client dashboards functions.

export type {
  Dashboard,
  DashboardPanel,
  DashboardWithPanels,
  CreateDashboardBody,
  DashboardUpdate,
} from "@pensieve-ai/client";

import type { CreateDashboardBody, DashboardUpdate } from "@pensieve-ai/client";
import { sessionClient } from "./client";

type Base = { endpoint: string; token: string; database?: string };

export function listDashboards(_args: Base) {
  return sessionClient().dashboards.listDashboards();
}

export function getDashboard(args: Base & { id: string }) {
  return sessionClient().dashboards.getDashboard({ id: args.id });
}

export function createDashboard(args: Base & { body: CreateDashboardBody }) {
  return sessionClient().dashboards.createDashboard({ body: args.body });
}

export function updateDashboard(args: Base & { id: string; patch: DashboardUpdate }) {
  return sessionClient().dashboards.updateDashboard({ id: args.id, patch: args.patch });
}

export function deleteDashboard(args: Base & { id: string }) {
  return sessionClient().dashboards.deleteDashboard({ id: args.id });
}

// Shim: backwards-compatible wrappers around @kyma-ai/client dashboards functions.

export type {
  Dashboard,
  DashboardPanel,
  DashboardWithPanels,
  CreateDashboardBody,
  DashboardUpdate,
} from "@kyma-ai/client";

import {
  listDashboards as _listDashboards,
  getDashboard as _getDashboard,
  createDashboard as _createDashboard,
  updateDashboard as _updateDashboard,
  deleteDashboard as _deleteDashboard,
  type CreateDashboardBody,
  type DashboardUpdate,
} from "@kyma-ai/client";
import { transportFor } from "./compat";

type Base = { endpoint: string; token: string; database?: string };

export function listDashboards(args: Base) {
  return _listDashboards(transportFor(args));
}

export function getDashboard(args: Base & { id: string }) {
  const { endpoint, token, database, id } = args;
  return _getDashboard(transportFor({ endpoint, token, database }), { id });
}

export function createDashboard(args: Base & { body: CreateDashboardBody }) {
  const { endpoint, token, database, body } = args;
  return _createDashboard(transportFor({ endpoint, token, database }), { body });
}

export function updateDashboard(args: Base & { id: string; patch: DashboardUpdate }) {
  const { endpoint, token, database, id, patch } = args;
  return _updateDashboard(transportFor({ endpoint, token, database }), { id, patch });
}

export function deleteDashboard(args: Base & { id: string }) {
  const { endpoint, token, database, id } = args;
  return _deleteDashboard(transportFor({ endpoint, token, database }), { id });
}

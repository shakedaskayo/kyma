// Shim: backwards-compatible wrappers around @kyma-ai/client data sources functions.

export type {
  DataSourceSummary,
  DataSourceDetail,
  CreateDataSourceBody,
  DataSourceUpdate,
  CatalogField,
  CatalogResource,
  CatalogEntry,
  GitHubRepo,
  DataSourceStatus,
} from "@kyma-ai/client";
export { SCHEDULE_MS_MIN, SCHEDULE_MS_MAX, deriveStatus } from "@kyma-ai/client";

import type { CreateDataSourceBody, DataSourceUpdate } from "@kyma-ai/client";
import { sessionClient } from "./client";

type Base = { endpoint: string; token: string; database?: string };

export function getDataSourceCatalog(_args: Base) {
  return sessionClient().datasources.getDataSourceCatalog();
}

export function listDataSources(_args: Base) {
  return sessionClient().datasources.listDataSources();
}

export function getDataSource(args: Base & { id: string }) {
  return sessionClient().datasources.getDataSource({ id: args.id });
}

export function createDataSource(args: Base & { body: CreateDataSourceBody }) {
  return sessionClient().datasources.createDataSource({ body: args.body });
}

export function patchDataSource(args: Base & { id: string; patch: DataSourceUpdate }) {
  return sessionClient().datasources.patchDataSource({ id: args.id, patch: args.patch });
}

export function deleteDataSource(args: Base & { id: string }) {
  return sessionClient().datasources.deleteDataSource({ id: args.id });
}

export function pauseDataSource(args: Base & { id: string }) {
  return sessionClient().datasources.pauseDataSource({ id: args.id });
}

export function resumeDataSource(args: Base & { id: string }) {
  return sessionClient().datasources.resumeDataSource({ id: args.id });
}

export function triggerDataSource(args: Base & { id: string }) {
  return sessionClient().datasources.triggerDataSource({ id: args.id });
}

export function listGitHubRepos(args: Base & { pat: string }) {
  return sessionClient().datasources.listGitHubRepos({ pat: args.pat });
}

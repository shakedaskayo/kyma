// Shim: backwards-compatible wrappers around @kyma-ai/client connectors functions.

export type {
  ConnectorSummary,
  ConnectorDetail,
  CreateConnectorBody,
  ConnectorUpdate,
  CatalogField,
  CatalogResource,
  CatalogEntry,
  GitHubRepo,
  ConnectorStatus,
} from "@kyma-ai/client";
export { SCHEDULE_MS_MIN, SCHEDULE_MS_MAX, deriveStatus } from "@kyma-ai/client";

import type { CreateConnectorBody, ConnectorUpdate } from "@kyma-ai/client";
import { sessionClient } from "./client";

type Base = { endpoint: string; token: string; database?: string };

export function getConnectorCatalog(_args: Base) {
  return sessionClient().connectors.getConnectorCatalog();
}

export function listConnectors(_args: Base) {
  return sessionClient().connectors.listConnectors();
}

export function getConnector(args: Base & { id: string }) {
  return sessionClient().connectors.getConnector({ id: args.id });
}

export function createConnector(args: Base & { body: CreateConnectorBody }) {
  return sessionClient().connectors.createConnector({ body: args.body });
}

export function patchConnector(args: Base & { id: string; patch: ConnectorUpdate }) {
  return sessionClient().connectors.patchConnector({ id: args.id, patch: args.patch });
}

export function deleteConnector(args: Base & { id: string }) {
  return sessionClient().connectors.deleteConnector({ id: args.id });
}

export function pauseConnector(args: Base & { id: string }) {
  return sessionClient().connectors.pauseConnector({ id: args.id });
}

export function resumeConnector(args: Base & { id: string }) {
  return sessionClient().connectors.resumeConnector({ id: args.id });
}

export function triggerConnector(args: Base & { id: string }) {
  return sessionClient().connectors.triggerConnector({ id: args.id });
}

export function listGitHubRepos(args: Base & { pat: string }) {
  return sessionClient().connectors.listGitHubRepos({ pat: args.pat });
}

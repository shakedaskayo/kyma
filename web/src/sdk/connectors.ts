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

import {
  getConnectorCatalog as _getConnectorCatalog,
  listConnectors as _listConnectors,
  getConnector as _getConnector,
  createConnector as _createConnector,
  patchConnector as _patchConnector,
  deleteConnector as _deleteConnector,
  pauseConnector as _pauseConnector,
  resumeConnector as _resumeConnector,
  triggerConnector as _triggerConnector,
  listGitHubRepos as _listGitHubRepos,
  type CreateConnectorBody,
  type ConnectorUpdate,
} from "@kyma-ai/client";
import { transportFor } from "./compat";

type Base = { endpoint: string; token: string; database?: string };

export function getConnectorCatalog(args: Base) {
  return _getConnectorCatalog(transportFor(args));
}

export function listConnectors(args: Base) {
  return _listConnectors(transportFor(args));
}

export function getConnector(args: Base & { id: string }) {
  const { endpoint, token, database, id } = args;
  return _getConnector(transportFor({ endpoint, token, database }), { id });
}

export function createConnector(args: Base & { body: CreateConnectorBody }) {
  const { endpoint, token, database, body } = args;
  return _createConnector(transportFor({ endpoint, token, database }), { body });
}

export function patchConnector(args: Base & { id: string; patch: ConnectorUpdate }) {
  const { endpoint, token, database, id, patch } = args;
  return _patchConnector(transportFor({ endpoint, token, database }), { id, patch });
}

export function deleteConnector(args: Base & { id: string }) {
  const { endpoint, token, database, id } = args;
  return _deleteConnector(transportFor({ endpoint, token, database }), { id });
}

export function pauseConnector(args: Base & { id: string }) {
  const { endpoint, token, database, id } = args;
  return _pauseConnector(transportFor({ endpoint, token, database }), { id });
}

export function resumeConnector(args: Base & { id: string }) {
  const { endpoint, token, database, id } = args;
  return _resumeConnector(transportFor({ endpoint, token, database }), { id });
}

export function triggerConnector(args: Base & { id: string }) {
  const { endpoint, token, database, id } = args;
  return _triggerConnector(transportFor({ endpoint, token, database }), { id });
}

export function listGitHubRepos(args: Base & { pat: string }) {
  const { endpoint, token, database, pat } = args;
  return _listGitHubRepos(transportFor({ endpoint, token, database }), { pat });
}

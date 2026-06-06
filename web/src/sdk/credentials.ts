// Shim: backwards-compatible wrappers around @kyma-ai/client credentials functions.

export type {
  CredentialKind,
  CredentialValue,
  CredentialSummary,
  CreateCredentialBody,
} from "@kyma-ai/client";
export { credentialKindLabel } from "@kyma-ai/client";

import {
  listCredentials as _listCredentials,
  createCredential as _createCredential,
  getCredential as _getCredential,
  deleteCredential as _deleteCredential,
  type CreateCredentialBody,
} from "@kyma-ai/client";
import { transportFor } from "./compat";

type Base = { endpoint: string; token: string; database?: string };

export function listCredentials(args: Base) {
  return _listCredentials(transportFor(args));
}

export function createCredential(args: Base & { body: CreateCredentialBody }) {
  const { endpoint, token, database, body } = args;
  return _createCredential(transportFor({ endpoint, token, database }), { body });
}

export function getCredential(args: Base & { id: string }) {
  const { endpoint, token, database, id } = args;
  return _getCredential(transportFor({ endpoint, token, database }), { id });
}

export function deleteCredential(args: Base & { id: string }) {
  const { endpoint, token, database, id } = args;
  return _deleteCredential(transportFor({ endpoint, token, database }), { id });
}

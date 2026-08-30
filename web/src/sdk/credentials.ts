// Shim: backwards-compatible wrappers around @pensieve-ai/client credentials functions.

export type {
  CredentialKind,
  CredentialValue,
  CredentialSummary,
  CreateCredentialBody,
} from "@pensieve-ai/client";
export { credentialKindLabel } from "@pensieve-ai/client";

import type { CreateCredentialBody } from "@pensieve-ai/client";
import { sessionClient } from "./client";

type Base = { endpoint: string; token: string; database?: string };

export function listCredentials(_args: Base) {
  return sessionClient().credentials.listCredentials();
}

export function createCredential(args: Base & { body: CreateCredentialBody }) {
  return sessionClient().credentials.createCredential({ body: args.body });
}

export function getCredential(args: Base & { id: string }) {
  return sessionClient().credentials.getCredential({ id: args.id });
}

export function deleteCredential(args: Base & { id: string }) {
  return sessionClient().credentials.deleteCredential({ id: args.id });
}

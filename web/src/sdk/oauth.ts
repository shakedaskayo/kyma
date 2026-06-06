// Shim: backwards-compatible wrappers around @kyma-ai/client oauth functions.

export type {
  StartOAuthBody,
  StartOAuthResult,
  OAuthFlowStatus,
} from "@kyma-ai/client";

import {
  startOAuth as _startOAuth,
  getOAuthFlowStatus as _getOAuthFlowStatus,
  type StartOAuthBody,
} from "@kyma-ai/client";
import { transportFor } from "./compat";

type Base = { endpoint: string; token: string; database?: string };

export function startOAuth(args: Base & { provider: string; body: StartOAuthBody }) {
  const { endpoint, token, database, provider, body } = args;
  return _startOAuth(transportFor({ endpoint, token, database }), { provider, body });
}

export function getOAuthFlowStatus(args: Base & { state: string }) {
  const { endpoint, token, database, state } = args;
  return _getOAuthFlowStatus(transportFor({ endpoint, token, database }), { state });
}

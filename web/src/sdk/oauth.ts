// Shim: backwards-compatible wrappers around @kyma-ai/client oauth functions.

export type {
  StartOAuthBody,
  StartOAuthResult,
  OAuthFlowStatus,
} from "@kyma-ai/client";

import type { StartOAuthBody } from "@kyma-ai/client";
import { sessionClient } from "./client";

type Base = { endpoint: string; token: string; database?: string };

export function startOAuth(args: Base & { provider: string; body: StartOAuthBody }) {
  return sessionClient().oauth.startOAuth({ provider: args.provider, body: args.body });
}

export function getOAuthFlowStatus(args: Base & { state: string }) {
  return sessionClient().oauth.getOAuthFlowStatus({ state: args.state });
}

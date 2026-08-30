// Shim: backwards-compatible wrappers around @pensieve-ai/client oauth functions.

export type {
  StartOAuthBody,
  StartOAuthResult,
  OAuthFlowStatus,
} from "@pensieve-ai/client";

import type { StartOAuthBody } from "@pensieve-ai/client";
import { sessionClient } from "./client";

type Base = { endpoint: string; token: string; database?: string };

export function startOAuth(args: Base & { provider: string; body: StartOAuthBody }) {
  return sessionClient().oauth.startOAuth({ provider: args.provider, body: args.body });
}

export function getOAuthFlowStatus(args: Base & { state: string }) {
  return sessionClient().oauth.getOAuthFlowStatus({ state: args.state });
}

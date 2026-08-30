// Shim: re-exports from @pensieve-ai/client with backwards-compatible web wrappers.
// login/refresh/getSetupStatus/signup stay unauthenticated — no change needed.
// me/logout are transport-based — delegate to sessionClient.

export {
  login,
  refresh,
  type AuthUser,
  type TokenPair,
} from "@pensieve-ai/client";

import { sessionClient } from "./client";

export async function me(_args: { endpoint: string; token: string }) {
  return sessionClient().auth.me();
}

export async function logout(_args: { endpoint: string; token: string }) {
  return sessionClient().auth.logout();
}

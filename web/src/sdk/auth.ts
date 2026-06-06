// Shim: re-exports from @kyma-ai/client with backwards-compatible web wrappers.
// login/refresh/getSetupStatus/signup stay unauthenticated — no change needed.
// me/logout are transport-based — wrapped to accept { endpoint, token }.

export {
  login,
  refresh,
  type AuthUser,
  type TokenPair,
} from "@kyma-ai/client";

import { me as _me, logout as _logout } from "@kyma-ai/client";
import { transportFor } from "./compat";

export async function me(args: { endpoint: string; token: string }) {
  return _me(transportFor(args));
}

export async function logout(args: { endpoint: string; token: string }) {
  return _logout(transportFor(args));
}

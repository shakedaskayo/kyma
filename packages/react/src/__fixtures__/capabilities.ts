import type { Capabilities } from "@kyma-ai/client";

export const CAPABILITIES_FIXTURE: Capabilities = {
  mode: "server",
  connectors: true,
  credentials: true,
  oauth: true,
  saved_views: true,
  users_admin: true,
  explore_live: true,
  agent: true,
};

import type { Capabilities } from "@pensieve-ai/client";

export const CAPABILITIES_FIXTURE: Capabilities = {
  mode: "server",
  data_sources: true,
  credentials: true,
  oauth: true,
  saved_views: true,
  users_admin: true,
  explore_live: true,
  agent: true,
};

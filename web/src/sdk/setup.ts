// Shim: re-exports from @pensieve-ai/client with backwards-compatible web wrappers.
// getSetupStatus/getSetupProbe/signup are unauthenticated — re-exported directly.
// ingestSample is transport-based — delegates to sessionClient.

export {
  getSetupStatus,
  getSetupProbe,
  signup,
  type SetupStatus,
  type SetupProbe,
  type EngineRecommendation,
} from "@pensieve-ai/client";

import { sessionClient } from "./client";

export async function ingestSample(args: {
  endpoint: string;
  token: string;
  database: string;
  table: string;
}) {
  return sessionClient().setup.ingestSample({ database: args.database, table: args.table });
}

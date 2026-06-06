// Shim: re-exports from @kyma-ai/client with backwards-compatible web wrappers.
// getSetupStatus/getSetupProbe/signup are unauthenticated — re-exported directly.
// ingestSample is transport-based — wrapped to accept { endpoint, token, ...args }.

export {
  getSetupStatus,
  getSetupProbe,
  signup,
  type SetupStatus,
  type SetupProbe,
  type EngineRecommendation,
} from "@kyma-ai/client";

import { ingestSample as _ingestSample } from "@kyma-ai/client";
import { transportFor } from "./compat";

export async function ingestSample(args: {
  endpoint: string;
  token: string;
  database: string;
  table: string;
}) {
  const { endpoint, token, database, table } = args;
  return _ingestSample(transportFor({ endpoint, token }), { database, table });
}

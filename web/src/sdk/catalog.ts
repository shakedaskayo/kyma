// Shim: backwards-compatible wrappers around @kyma-ai/client catalog functions.

export type {
  ColumnInfo,
  TableDoc,
  DatabaseDoc,
  SchemaDoc,
} from "@kyma-ai/client";

import { sessionClient } from "./client";

type Base = { endpoint: string; token: string; database?: string };

export function fetchSchema(_args: Base) {
  return sessionClient().catalog.fetchSchema();
}

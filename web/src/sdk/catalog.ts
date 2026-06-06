// Shim: backwards-compatible wrappers around @kyma-ai/client catalog functions.

export type {
  ColumnInfo,
  TableDoc,
  DatabaseDoc,
  SchemaDoc,
} from "@kyma-ai/client";

import { fetchSchema as _fetchSchema } from "@kyma-ai/client";
import { transportFor } from "./compat";

type Base = { endpoint: string; token: string; database?: string };

export function fetchSchema(args: Base) {
  return _fetchSchema(transportFor(args));
}

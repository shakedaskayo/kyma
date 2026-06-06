// Shim: backwards-compatible wrappers around @kyma-ai/client query functions.

export type { Column, ColKind, ResultChunk, QueryArgs } from "@kyma-ai/client";

import { runQuery as _runQuery, type QueryArgs } from "@kyma-ai/client";
import { transportFor } from "./compat";

type Base = { endpoint: string; token: string };

export function runQuery(
  args: Base & QueryArgs,
): ReturnType<typeof _runQuery> {
  const { endpoint, token, ...rest } = args;
  return _runQuery(transportFor({ endpoint, token }), rest);
}

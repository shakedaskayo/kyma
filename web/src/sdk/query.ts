// Shim: backwards-compatible wrappers around @kyma-ai/client query functions.

export type { Column, ColKind, ResultChunk, QueryArgs } from "@kyma-ai/client";

import type { QueryArgs, ResultChunk } from "@kyma-ai/client";
import { sessionClient } from "./client";

type Base = { endpoint: string; token: string };

export function runQuery(
  args: Base & QueryArgs,
): AsyncGenerator<ResultChunk, void, void> {
  // endpoint/token from args are ignored — sessionClient uses the live session.
  const { endpoint: _ep, token: _tok, ...rest } = args;
  return sessionClient().query.runQuery(rest as QueryArgs);
}

// Transitional: builds a one-shot transport from the session args shape the
// app still passes around. Phase 6 replaces this with a session-level client.
import { createTransport, type KymaTransport } from "@kyma-ai/client";

export function transportFor(args: { endpoint: string; token: string; database?: string }): KymaTransport {
  return createTransport({ endpoint: args.endpoint, auth: { token: args.token }, database: args.database });
}

// Shim: backwards-compatible wrappers around @kyma-ai/client discover functions.
// All session calls delegate to sessionClient() — no one-shot transports.

export type {
  Frame,
  Scope,
  SearchRequest,
  SavedView,
} from "@kyma-ai/client";
export { DiscoverError, parseNdjsonStream } from "@kyma-ai/client";

import type { SearchRequest } from "@kyma-ai/client";
import { sessionClient } from "./client";

// ── Session-implicit functions ────────────────────────────────────────────────

// Old signature: searchDiscover(req, signal?)
export function searchDiscover(req: SearchRequest, signal?: AbortSignal) {
  return sessionClient().discover.searchDiscover(req, signal);
}

// Old signature: listSavedViews() — no args
export function listSavedViews() {
  return sessionClient().discover.listSavedViews();
}

// Old signature: createSavedView(name, sources)
export function createSavedView(name: string, sources: string[]) {
  return sessionClient().discover.createSavedView(name, sources);
}

// Old signature: deleteSavedView(id)
export function deleteSavedView(id: string) {
  return sessionClient().discover.deleteSavedView(id);
}

// Shim: backwards-compatible wrappers around @kyma-ai/client discover functions.
// In the old web/src/sdk, many functions used authFetch internally (no session args).
// We replicate that by reading session state at call time via useSession.getState().

export type {
  Frame,
  Scope,
  SearchRequest,
  SavedView,
} from "@kyma-ai/client";
export { DiscoverError, parseNdjsonStream } from "@kyma-ai/client";

import {
  searchDiscover as _searchDiscover,
  listSavedViews as _listSavedViews,
  createSavedView as _createSavedView,
  deleteSavedView as _deleteSavedView,
  type SearchRequest,
} from "@kyma-ai/client";
import { useSession } from "./session";
import { transportFor } from "./compat";

// ── Session-implicit functions (old API used authFetch, no args) ─────────────

function sessionTransport() {
  const { endpoint, token, database } = useSession.getState();
  return transportFor({ endpoint, token, database });
}

// Old signature: searchDiscover(req, signal?)
export function searchDiscover(req: SearchRequest, signal?: AbortSignal) {
  return _searchDiscover(sessionTransport(), req, signal);
}

// Old signature: listSavedViews() — no args
export function listSavedViews() {
  return _listSavedViews(sessionTransport());
}

// Old signature: createSavedView(name, sources) — no session args
export function createSavedView(name: string, sources: string[]) {
  return _createSavedView(sessionTransport(), name, sources);
}

// Old signature: deleteSavedView(id) — no session args
export function deleteSavedView(id: string) {
  return _deleteSavedView(sessionTransport(), id);
}

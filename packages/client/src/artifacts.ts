// Object-store artifact fetch (`/v1/artifacts/by-path`) — read a byte window of
// a stored artifact (e.g. a CI job log captured by the GitHub data source) by its
// object-store key. Graph `LogFile` nodes carry an `object_path` (not the
// artifact id), so the graph log-viewer fetches content through this path-keyed
// endpoint. Range reads (`offset`/`limit`) let the UI page through large logs;
// the server caps the window and reports `eof` when the tail is reached.

import { errorFromResponse } from "./errors";
import type { PensieveTransport } from "./transport";

export interface ArtifactWindow {
  /** Echoed object-store key. */
  object_path: string;
  /** Total artifact size in bytes (the full object, not this window). */
  size_bytes: number;
  /** Byte offset this window starts at. */
  offset: number;
  /** Bytes returned in `content` (after lossy UTF-8 decode this may differ in length). */
  returned_bytes: number;
  /** True when `offset + returned_bytes` reaches the end of the artifact. */
  eof: boolean;
  /** The decoded window text. */
  content: string;
}

/**
 * Fetch a byte window of a stored artifact by its object-store key (e.g. a
 * `LogFile` graph node's `object_path`). Omitting `limit` uses the server
 * default window (64 KiB); the server clamps oversized requests.
 */
export async function fetchArtifactByPath(
  t: PensieveTransport,
  path: string,
  opts?: { offset?: number; limit?: number },
): Promise<ArtifactWindow> {
  const params = new URLSearchParams({ path });
  if (opts?.offset != null) params.set("offset", String(opts.offset));
  if (opts?.limit != null) params.set("limit", String(opts.limit));
  const res = await t.request(`/v1/artifacts/by-path?${params}`);
  if (!res.ok) throw await errorFromResponse(res);
  return (await res.json()) as ArtifactWindow;
}

//! Typed client for data-retention settings (`/v1/agent/retention/settings`)
//! and object-store artifact fetch-by-path (`/v1/artifacts/by-path`).

type Args = { endpoint: string; token: string };

function base(endpoint: string) {
  return endpoint.replace(/\/$/, "");
}

/** Mirror of `pensieve_core::retention::RetentionSettings`. Day-counts delete data
 *  older than N days; `null`/absent = retain forever. */
export interface RetentionSettings {
  global_default_days: number | null;
  per_source_days: Record<string, number>;
  per_table_days: Record<string, number>;
  per_artifact_class_days: Record<string, number>;
}

export const EMPTY_RETENTION: RetentionSettings = {
  global_default_days: null,
  per_source_days: {},
  per_table_days: {},
  per_artifact_class_days: {},
};

export async function getRetentionSettings(a: Args): Promise<RetentionSettings> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/retention/settings`, {
    headers: {
      authorization: `Bearer ${a.token}`,
      "content-type": "application/json",
    },
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`retention settings: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  // The backend omits empty maps (serde default) — normalize to a full shape.
  return { ...EMPTY_RETENTION, ...(await res.json()) };
}

export async function putRetentionSettings(a: Args, s: RetentionSettings): Promise<void> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/retention/settings`, {
    method: "PUT",
    headers: {
      authorization: `Bearer ${a.token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(s),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`retention settings: ${res.status}${t ? ` — ${t}` : ""}`);
  }
}

export interface ArtifactWindow {
  object_path: string;
  size_bytes: number;
  offset: number;
  returned_bytes: number;
  eof: boolean;
  content: string;
}

/** Fetch a byte window of a stored artifact by its object-store key (e.g. a
 *  `LogFile` graph node's `object_path`). */
export async function fetchArtifactByPath(
  a: Args,
  path: string,
  opts?: { offset?: number; limit?: number },
): Promise<ArtifactWindow> {
  const params = new URLSearchParams({ path });
  if (opts?.offset != null) params.set("offset", String(opts.offset));
  if (opts?.limit != null) params.set("limit", String(opts.limit));
  const res = await fetch(`${base(a.endpoint)}/v1/artifacts/by-path?${params}`, {
    headers: { authorization: `Bearer ${a.token}` },
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`artifact: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  return res.json();
}

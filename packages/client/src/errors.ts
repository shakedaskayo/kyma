/** Error from a Kyma API response. `status` is the HTTP status. */
export class KymaApiError extends Error {
  readonly status: number;
  readonly requestId?: string;
  constructor(status: number, message: string, requestId?: string) {
    super(message);
    this.name = "KymaApiError";
    this.status = status;
    this.requestId = requestId;
  }
}

/** 401/403 — token invalid, expired beyond refresh, or insufficient scope. */
export class KymaAuthError extends KymaApiError {
  constructor(status: number, message: string, requestId?: string) {
    super(status, message, requestId);
    this.name = "KymaAuthError";
  }
}

export async function errorFromResponse(res: Response): Promise<KymaApiError> {
  const requestId = res.headers.get("x-request-id") ?? undefined;
  let detail = "";
  try {
    const text = await res.text();
    if ((res.headers.get("content-type") ?? "").includes("application/json")) {
      const body = JSON.parse(text) as { error?: string; message?: string };
      detail = body.error ?? body.message ?? text;
    } else {
      detail = text;
    }
  } catch {
    /* body unreadable — fall through to status-only message */
  }
  detail = detail.slice(0, 200);
  const message = `kyma request failed: ${res.status}${detail ? ` — ${detail}` : ""}`;
  return res.status === 401 || res.status === 403
    ? new KymaAuthError(res.status, message, requestId)
    : new KymaApiError(res.status, message, requestId);
}

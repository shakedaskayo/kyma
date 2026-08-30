export interface PensieveApiErrorOpts {
  requestId?: string;
  code?: string;
}

/** Error from a Pensieve API response. `status` is the HTTP status. */
export class PensieveApiError extends Error {
  readonly status: number;
  readonly requestId?: string;
  readonly code?: string;
  constructor(status: number, message: string, opts?: PensieveApiErrorOpts) {
    super(message);
    this.name = "PensieveApiError";
    this.status = status;
    this.requestId = opts?.requestId;
    this.code = opts?.code;
  }
}

/** 401/403 — token invalid, expired beyond refresh, or insufficient scope. */
export class PensieveAuthError extends PensieveApiError {
  constructor(status: number, message: string, opts?: PensieveApiErrorOpts) {
    super(status, message, opts);
    this.name = "PensieveAuthError";
  }
}

export async function errorFromResponse(res: Response): Promise<PensieveApiError> {
  const requestId = res.headers.get("x-request-id") ?? undefined;
  let detail = "";
  let code: string | undefined;
  try {
    const text = await res.text();
    if ((res.headers.get("content-type") ?? "").includes("application/json")) {
      const body = JSON.parse(text) as { error?: string; message?: string; code?: string };
      detail = body.error ?? body.message ?? text;
      code = typeof body.code === "string" ? body.code : undefined;
    } else {
      detail = text;
    }
  } catch {
    /* body unreadable — fall through to status-only message */
  }
  detail = detail.slice(0, 200);
  const isAuth = res.status === 401 || res.status === 403;
  // For auth failures, include "unauthorized" so callers testing /unauthorized/i match.
  const statusLabel = isAuth ? `${res.status} unauthorized` : `${res.status}`;
  const message = `pensieve request failed: ${statusLabel}${detail ? ` — ${detail}` : ""}`;
  const opts: PensieveApiErrorOpts = { requestId, code };
  return isAuth
    ? new PensieveAuthError(res.status, message, opts)
    : new PensieveApiError(res.status, message, opts);
}

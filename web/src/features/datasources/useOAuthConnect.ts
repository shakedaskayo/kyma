import { useCallback, useEffect, useRef, useState } from "react";
import { useSession } from "@/sdk/session";
import { getOAuthFlowStatus, startOAuth, type StartOAuthBody } from "@/sdk/oauth";

export type OAuthPhase = "idle" | "starting" | "awaiting" | "success" | "error";

export interface ConnectedAccount {
  credentialId: string;
  label?: string;
}

/**
 * Drives a "Connect <provider>" flow: starts the flow, opens the provider
 * authorize page in a popup, and resolves the minted credential via a
 * `postMessage` handshake (validated by origin + state). Falls back to polling
 * `/v1/oauth/flows/:state` (and an "open in a new tab" link) when the popup is
 * blocked or postMessage can't reach the SPA (e.g. a desktop webview). The
 * bring-your-own client secret is passed as a `connect()` argument — never
 * stored in a query cache key.
 */
export function useOAuthConnect(provider: string | undefined, dataSourceType: string) {
  const { endpoint, token, database } = useSession();
  const [phase, setPhase] = useState<OAuthPhase>("idle");
  const [account, setAccount] = useState<ConnectedAccount | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [popupBlocked, setPopupBlocked] = useState(false);
  const [authorizeUrl, setAuthorizeUrl] = useState<string | null>(null);

  const stateRef = useRef<string | null>(null);
  const popupRef = useRef<Window | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const closeWatchRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const clearTimers = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
    if (closeWatchRef.current) {
      clearInterval(closeWatchRef.current);
      closeWatchRef.current = null;
    }
  }, []);

  const cleanup = useCallback(() => {
    clearTimers();
    try {
      popupRef.current?.close();
    } catch {
      /* ignore */
    }
    popupRef.current = null;
  }, [clearTimers]);

  const reset = useCallback(() => {
    cleanup();
    stateRef.current = null;
    setPhase("idle");
    setAccount(null);
    setError(null);
    setPopupBlocked(false);
    setAuthorizeUrl(null);
  }, [cleanup]);

  const succeed = useCallback(
    (credentialId: string, label?: string) => {
      cleanup();
      setAccount({ credentialId, label });
      setError(null);
      setPopupBlocked(false);
      setPhase("success");
    },
    [cleanup],
  );

  const fail = useCallback(
    (msg: string) => {
      cleanup();
      setError(msg);
      setPhase("error");
    },
    [cleanup],
  );

  const startPolling = useCallback(() => {
    if (pollRef.current) return;
    pollRef.current = setInterval(() => {
      const st = stateRef.current;
      if (!st) return;
      getOAuthFlowStatus({ endpoint, token, database, state: st })
        .then((res) => {
          if (res.status === "completed" && res.credential_id) succeed(res.credential_id);
          else if (res.status === "error") fail("Authorization was not completed.");
        })
        .catch(() => {
          /* transient; keep polling */
        });
    }, 1500);
  }, [endpoint, token, database, succeed, fail]);

  // Listen for the callback's postMessage. Only trust the engine origin + our
  // own state token.
  useEffect(() => {
    function onMessage(ev: MessageEvent) {
      let originOk = false;
      try {
        originOk = ev.origin === new URL(endpoint).origin;
      } catch {
        originOk = false;
      }
      if (!originOk) return;
      const data = ev.data as
        | { type?: string; ok?: boolean; state?: string; credential_id?: string; label?: string; error?: string }
        | undefined;
      if (!data || data.type !== "kyma-oauth") return;
      if (stateRef.current && data.state && data.state !== stateRef.current) return;
      if (data.ok && data.credential_id) succeed(data.credential_id, data.label);
      else fail(data.error || "Authorization failed.");
    }
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [endpoint, succeed, fail]);

  // Clean up timers/popup on unmount.
  useEffect(() => cleanup, [cleanup]);

  const connect = useCallback(
    async (opts?: { clientId?: string; clientSecret?: string; scopes?: string[]; label?: string }) => {
      if (!provider) {
        fail("This data source has no OAuth provider configured.");
        return;
      }
      setPhase("starting");
      setError(null);
      setAccount(null);
      setPopupBlocked(false);
      try {
        const body: StartOAuthBody = { data_source_type: dataSourceType };
        if (opts?.scopes?.length) body.scopes = opts.scopes;
        if (opts?.label) body.label = opts.label;
        if (opts?.clientId && opts?.clientSecret) {
          body.client_id = opts.clientId;
          body.client_secret = opts.clientSecret;
        }
        const { authorize_url, state } = await startOAuth({
          endpoint,
          token,
          database,
          provider,
          body,
        });
        stateRef.current = state;
        setAuthorizeUrl(authorize_url);
        setPhase("awaiting");

        const popup = window.open(authorize_url, "kyma-oauth", "width=620,height=760");
        if (!popup) {
          // Popup blocked (or a webview that won't open one) → poll for the
          // result and surface an "open in a new tab" link to the caller.
          setPopupBlocked(true);
          startPolling();
          return;
        }
        popupRef.current = popup;
        closeWatchRef.current = setInterval(() => {
          if (popupRef.current?.closed) {
            if (closeWatchRef.current) {
              clearInterval(closeWatchRef.current);
              closeWatchRef.current = null;
            }
            // The message may still be in flight; poll a few times before giving up.
            startPolling();
          }
        }, 800);
      } catch (e) {
        fail((e as Error).message);
      }
    },
    [endpoint, token, database, provider, dataSourceType, startPolling, fail],
  );

  return { phase, account, error, popupBlocked, authorizeUrl, connect, reset };
}

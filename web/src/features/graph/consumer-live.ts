/**
 * Dedicated WebSocket client for `GET /v1/consumers/live`.
 *
 * Modeled on the discover `LiveSession` (auth-by-first-message, reconnect with
 * backoff, no reconnect on 1008) but pointed at the consumers endpoint with its
 * own `type`-tagged frame schema. Not reused from `LiveSession` because that
 * hardcodes the explore path + an explore-search subscribe body.
 */

export type ConsumerLiveStatus = "connecting" | "live" | "closed" | "error";

/** Wire shape of a single activity (mirrors the Rust `ConsumerActivity`). */
export interface ConsumerActivityFrame {
  consumer_id: string;
  kind: string;
  label: string;
  subject: string | null;
  tenant: string;
  action: string; // "recall" | "search" | "remember"
  node_ids: string[];
  namespaces: string[];
  query_preview: string | null;
  ts: number;
  host?: string | null;
  client_version?: string | null;
  transport?: string | null;
  ip?: string | null;
  pid?: number | null;
}

type ConsumerFrame =
  | { type: "backfill"; activity: ConsumerActivityFrame }
  | { type: "live" }
  | { type: "events"; events: ConsumerActivityFrame[] }
  | { type: "heartbeat" }
  | { type: "lagged" }
  | { type: "error"; code?: string; message?: string };

type Opts = {
  endpoint: string;
  token: string;
  /** Called per activity. `backfill` is true for connect-time history. */
  onActivity: (a: ConsumerActivityFrame, backfill: boolean) => void;
  onStatus: (s: ConsumerLiveStatus) => void;
  wsFactory?: (url: string) => WebSocket;
};

const BACKOFF_INITIAL_MS = 1_000;
const BACKOFF_MAX_MS = 30_000;
const NO_RECONNECT_CODES = new Set([1008]); // policy violation — bad token

export class ConsumerLiveSession {
  private readonly opts: Opts;
  private readonly wsUrl: string;
  private ws: WebSocket | null = null;
  private closed = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private backoffMs = BACKOFF_INITIAL_MS;

  constructor(opts: Opts) {
    this.opts = opts;
    this.wsUrl =
      opts.endpoint.replace(/\/$/, "").replace(/^http/, "ws") + "/v1/consumers/live";
    this.opts.onStatus("connecting");
    this.connect();
  }

  private connect() {
    if (this.closed) return;
    const factory = this.opts.wsFactory ?? ((url: string) => new WebSocket(url));
    const ws = factory(this.wsUrl);
    this.ws = ws;

    ws.onopen = () => {
      if (this.closed || ws !== this.ws) {
        ws.close();
        return;
      }
      ws.send(JSON.stringify({ type: "auth", token: this.opts.token }));
    };

    ws.onmessage = (ev: MessageEvent) => {
      if (this.closed) return;
      let frame: ConsumerFrame;
      try {
        frame = JSON.parse(ev.data as string) as ConsumerFrame;
      } catch {
        return; // malformed — ignore, don't kill the session
      }
      switch (frame.type) {
        case "backfill":
          this.opts.onActivity(frame.activity, true);
          break;
        case "live":
          this.backoffMs = BACKOFF_INITIAL_MS;
          this.opts.onStatus("live");
          break;
        case "events":
          for (const a of frame.events) this.opts.onActivity(a, false);
          break;
        case "heartbeat":
        case "lagged":
          break; // liveness only; the UI re-renders from its own ring
        case "error":
          // Coded error before close — surface as error; close handles retry.
          this.opts.onStatus("error");
          break;
      }
    };

    ws.onclose = (ev: CloseEvent) => {
      if (this.closed || ws !== this.ws) return;
      if (NO_RECONNECT_CODES.has(ev.code)) {
        this.opts.onStatus("error");
        return;
      }
      this.scheduleReconnect();
    };

    ws.onerror = () => {
      // onclose fires after onerror; reconnect is handled there.
    };
  }

  private scheduleReconnect() {
    if (this.closed || this.reconnectTimer !== null) return;
    const delay = this.backoffMs;
    this.backoffMs = Math.min(this.backoffMs * 2, BACKOFF_MAX_MS);
    this.opts.onStatus("connecting");
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      if (!this.closed) this.connect();
    }, delay);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      this.ws.close(1000, "session closed");
      this.ws = null;
    }
    this.opts.onStatus("closed");
  }
}

#!/usr/bin/env node
// Smoke test for the Discover live WebSocket: auth → subscribe → backfill →
// live marker → ingest a row → assert it arrives as an incremental frame.
//
// Usage: node scripts/discover-live-smoke.mjs [endpoint] [token]
//   endpoint defaults to http://127.0.0.1:8080
//   token defaults to $KYMA_TOKEN or the local CLI config (~/.kyma/config.json)
//
// Requires Node >= 22 (built-in WebSocket). Exits 0 on success.

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const endpoint = process.argv[2] ?? "http://127.0.0.1:8080";
const token =
  process.argv[3] ??
  process.env.KYMA_TOKEN ??
  JSON.parse(readFileSync(join(homedir(), ".kyma", "config.json"), "utf8")).token;

const wsUrl = endpoint.replace(/\/$/, "").replace(/^http/, "ws") + "/v1/explore/live";
const die = (msg) => {
  console.error(`✗ ${msg}`);
  process.exit(1);
};
const ok = (msg) => console.log(`✓ ${msg}`);

const marker = `live-smoke-${Date.now()}`;
let sawLive = false;
let backfillFrames = 0;

const ws = new WebSocket(wsUrl);
const timeout = setTimeout(() => die("timed out waiting for the live row (10s)"), 10_000);

ws.onopen = () => {
  ws.send(JSON.stringify({ type: "auth", token }));
  ws.send(
    JSON.stringify({
      type: "subscribe",
      query: "",
      scope: { kind: "sources", sources: ["demo.*"] },
      time_range: null,
      per_source_limit: 50,
    }),
  );
};

ws.onmessage = async (ev) => {
  const frame = JSON.parse(ev.data);
  if (!sawLive) {
    if (frame.type === "live") {
      sawLive = true;
      ok(`backfill complete (${backfillFrames} frames) — session is live`);
      // Now ingest a marker row and wait for it to come back over the socket.
      const res = await fetch(`${endpoint}/v1/ingest`, {
        method: "POST",
        headers: {
          authorization: `Bearer ${token}`,
          "content-type": "application/json",
          "x-database": "demo",
          "x-table": "events",
        },
        body: JSON.stringify({
          timestamp: new Date().toISOString(),
          message: marker,
          service: "smoke",
          severity: "INFO",
        }),
      });
      if (!res.ok) die(`ingest failed: HTTP ${res.status}`);
      ok("marker row ingested — waiting for it on the socket…");
    } else {
      backfillFrames++;
      if (frame.type === "error") die(`error frame during backfill: ${JSON.stringify(frame)}`);
    }
    return;
  }
  if (frame.type === "rows") {
    const hit = frame.rows.find((r) => r.message === marker);
    if (hit) {
      clearTimeout(timeout);
      ok(`marker row received live from ${frame.source} 🔴`);
      ws.close();
      process.exit(0);
    }
  }
  if (frame.type === "error") die(`error frame while live: ${JSON.stringify(frame)}`);
};

ws.onclose = (ev) => {
  if (!sawLive) die(`socket closed before live (code ${ev.code} ${ev.reason})`);
};
ws.onerror = () => die("websocket error (is the server running?)");

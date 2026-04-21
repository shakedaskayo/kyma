import { createGrpcWebTransport } from "@connectrpc/connect-web";
import { createPromiseClient } from "@connectrpc/connect";
import { FlightService } from "./gen/Flight_connect";
import { Ticket, FlightData } from "./gen/Flight_pb";
import { tableFromIPC, RecordBatch } from "apache-arrow";

export type QueryArgs = {
  endpoint: string;
  token: string;
  database: string;
  query: string;
  language: "kql" | "sql";
  walMs?: number;
  memBytes?: number;
  signal?: AbortSignal;
};

export function encodeTicket(t: { database: string; query: string; language: string }): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(t));
}

export async function* runQuery(args: QueryArgs): AsyncGenerator<RecordBatch, void, void> {
  const transport = createGrpcWebTransport({ baseUrl: args.endpoint.replace(/\/$/, "") });
  const client = createPromiseClient(FlightService, transport);

  const ticket = new Ticket({ ticket: encodeTicket({
    database: args.database, query: args.query, language: args.language,
  }) });

  const headers: Record<string, string> = { authorization: `Bearer ${args.token}` };
  if (args.walMs)    headers["x-kyma-max-wall-clock-ms"] = String(args.walMs);
  if (args.memBytes) headers["x-kyma-max-memory-bytes"]  = String(args.memBytes);

  const stream = client.doGet(ticket, { headers, signal: args.signal });

  // Re-frame each FlightData (raw flatbuffer + body) back into Arrow IPC stream
  // format so apache-arrow's tableFromIPC can decode it. tonic's
  // FlightDataEncoder strips the stream's continuation/length/padding; we put
  // it back here on the wire side.
  const chunks: Uint8Array[] = [];
  for await (const msg of stream as AsyncIterable<FlightData>) {
    for (const part of reframe(msg)) chunks.push(part);
  }
  // IPC stream end-of-stream marker: 4-byte continuation + 4-byte zero length.
  chunks.push(endOfStream());

  const full = concat(chunks);
  const table = tableFromIPC(full);
  for (const batch of table.batches) yield batch;
}

export function reframe(msg: FlightData): Uint8Array[] {
  const out: Uint8Array[] = [];
  if (msg.dataHeader.length) {
    const headerLen = msg.dataHeader.length;
    const padded = (headerLen + 7) & ~7; // 8-byte align
    const prefix = new Uint8Array(8);
    const view = new DataView(prefix.buffer);
    view.setInt32(0, -1, true);          // continuation: 0xFFFFFFFF
    view.setInt32(4, padded, true);      // metadata length (includes padding)
    out.push(prefix);
    out.push(msg.dataHeader);
    if (padded > headerLen) out.push(new Uint8Array(padded - headerLen));
  }
  if (msg.dataBody.length) out.push(msg.dataBody);
  return out;
}

export function endOfStream(): Uint8Array {
  const buf = new Uint8Array(8);
  const view = new DataView(buf.buffer);
  view.setInt32(0, -1, true); // continuation
  view.setInt32(4, 0, true);  // zero length → EOS
  return buf;
}

function concat(xs: Uint8Array[]): Uint8Array {
  const total = xs.reduce((n, x) => n + x.length, 0);
  const out = new Uint8Array(total);
  let o = 0;
  for (const x of xs) { out.set(x, o); o += x.length; }
  return out;
}

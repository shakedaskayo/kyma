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

  const ticket = new Ticket({
    ticket: encodeTicket({
      database: args.database,
      query: args.query,
      language: args.language,
    }),
  });

  const headers: Record<string, string> = { authorization: `Bearer ${args.token}` };
  if (args.walMs) headers["x-kyma-max-wall-clock-ms"] = String(args.walMs);
  if (args.memBytes) headers["x-kyma-max-memory-bytes"] = String(args.memBytes);

  const stream = client.doGet(ticket, { headers, signal: args.signal });

  // Each FlightData message carries a chunk of the Arrow IPC stream.
  // The server's FlightDataEncoder emits messages whose concatenated
  // data_header + data_body form a valid Arrow IPC stream.
  // MVP buffers all messages then decodes.
  const chunks: Uint8Array[] = [];
  for await (const msg of stream as AsyncIterable<FlightData>) {
    if (msg.dataHeader.length) chunks.push(msg.dataHeader);
    if (msg.dataBody.length) chunks.push(msg.dataBody);
  }
  const full = concat(chunks);
  const table = tableFromIPC(full);
  for (const batch of table.batches) yield batch;
}

function concat(xs: Uint8Array[]): Uint8Array {
  const total = xs.reduce((n, x) => n + x.length, 0);
  const out = new Uint8Array(total);
  let o = 0;
  for (const x of xs) {
    out.set(x, o);
    o += x.length;
  }
  return out;
}

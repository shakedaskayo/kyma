import { expect, test } from "vitest";
import { encodeQueryState, decodeQueryState } from "./url-state";

test("round-trips a query + time range", () => {
  const s = { query: "otel_logs | take 5", preset: "1h", from: undefined, to: undefined } as const;
  const encoded = encodeQueryState(s);
  expect(encoded.length).toBeGreaterThan(0);
  const got = decodeQueryState(encoded);
  expect(got).toEqual(s);
});

test("decode returns null for garbage", () => {
  expect(decodeQueryState("not-a-valid-base64!!")).toBeNull();
});

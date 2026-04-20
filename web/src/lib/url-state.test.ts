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

test("decode returns null for valid base64/json but wrong shape", () => {
  // Valid base64url JSON: { "foo": 1 } — missing required fields.
  const bogus = "eyJmb28iOjF9"; // base64url of '{"foo":1}'
  expect(decodeQueryState(bogus)).toBeNull();
});

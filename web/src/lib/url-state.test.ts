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

test("decode accepts valid preset", () => {
  const s = { query: "t | take 1", preset: "1h" as const };
  const encoded = encodeQueryState(s);
  const got = decodeQueryState(encoded);
  expect(got).not.toBeNull();
  expect(got?.preset).toBe("1h");
});

test("decode returns null for invalid preset", () => {
  // Encode a state with a bogus preset string, bypassing TypeScript types.
  const s = { query: "t | take 1", preset: "bogus" };
  const json = JSON.stringify(s);
  const utf8 = new TextEncoder().encode(json);
  let bin = "";
  for (let i = 0; i < utf8.length; i++) bin += String.fromCharCode(utf8[i]);
  const encoded = btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  expect(decodeQueryState(encoded)).toBeNull();
});

test("decode returns null for missing preset field", () => {
  const s = { query: "t | take 1" }; // no preset
  const json = JSON.stringify(s);
  const utf8 = new TextEncoder().encode(json);
  let bin = "";
  for (let i = 0; i < utf8.length; i++) bin += String.fromCharCode(utf8[i]);
  const encoded = btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  expect(decodeQueryState(encoded)).toBeNull();
});

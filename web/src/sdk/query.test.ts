// web/src/sdk/query.test.ts — smoke-level; integration test covered in E2E.
import { expect, test } from "vitest";
import { encodeTicket } from "./query";

test("encodeTicket serializes database+query+language as JSON", () => {
  const bytes = encodeTicket({ database: "obs", query: "otel_logs | take 1", language: "kql" });
  const text = new TextDecoder().decode(bytes);
  const json = JSON.parse(text);
  expect(json).toEqual({ database: "obs", query: "otel_logs | take 1", language: "kql" });
});

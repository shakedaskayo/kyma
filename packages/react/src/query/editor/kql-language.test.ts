import { expect, test } from "vitest";
import { KQL_LANG_ID, registerKql, registerKqlCompletions } from "./kql-language";

test("KQL_LANG_ID is 'kql'", () => {
  expect(KQL_LANG_ID).toBe("kql");
});

test("registerKql and registerKqlCompletions are exported functions", () => {
  // The Monaco singleton is unavailable in vitest (no WASM). We just verify
  // the module exports the correct shapes so a broken import refactor fails
  // loudly during CI.
  expect(typeof registerKql).toBe("function");
  expect(typeof registerKqlCompletions).toBe("function");
});

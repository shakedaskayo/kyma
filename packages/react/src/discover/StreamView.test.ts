import { expect, test } from "vitest";
import { shouldFreeze, bufferedCount } from "./StreamView";

test("shouldFreeze returns false when scrollTop is 0 (at top)", () => {
  expect(shouldFreeze(0)).toBe(false);
});

test("shouldFreeze returns true when scrollTop > 0 (scrolled down)", () => {
  expect(shouldFreeze(1)).toBe(true);
  expect(shouldFreeze(200)).toBe(true);
});

test("bufferedCount returns difference when new rows arrive while frozen", () => {
  expect(bufferedCount(10, 15)).toBe(5);
});

test("bufferedCount returns 0 when no new rows", () => {
  expect(bufferedCount(10, 10)).toBe(0);
});

test("bufferedCount returns 0 when nextLen < prevLen (should not happen, but safe)", () => {
  expect(bufferedCount(10, 8)).toBe(0);
});

test("bufferedCount handles zero prev length (first freeze)", () => {
  expect(bufferedCount(0, 5)).toBe(5);
});

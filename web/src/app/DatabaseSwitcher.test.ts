import { expect, test } from "vitest";
import { headerScopeFor } from "./DatabaseSwitcher";

test("the Query Editor (and legacy /explore) is cross-database with an All-databases scope", () => {
  for (const p of ["/query", "/query/abc", "/explore"]) {
    expect(headerScopeFor(p).mode, p).toBe("query");
  }
});

test("single-database pages show the concrete switcher", () => {
  for (const p of [
    "/agent",
    "/data-sources",
    "/data-sources/123",
    "/dashboards",
    "/dashboards/42",
  ]) {
    expect(headerScopeFor(p).mode, p).toBe("switch");
  }
});

test("graph is cross-database, so it shows a read-only 'All databases' chip", () => {
  const scope = headerScopeFor("/graph");
  expect(scope.mode).toBe("all");
  if (scope.mode === "all") {
    expect(scope.label).toBe("All databases");
    expect(scope.reason.length).toBeGreaterThan(0);
  }
});

test("pages with their own / no database scope hide the header chip", () => {
  for (const p of ["/discover", "/credentials", "/settings", "/"]) {
    expect(headerScopeFor(p).mode, p).toBe("hidden");
  }
});

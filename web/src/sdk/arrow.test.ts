import { expect, test } from "vitest";
import { tableFromArrays } from "apache-arrow";
import { batchToRows, autoChartAxes } from "./arrow";

test("batchToRows extracts columns + rows", () => {
  const table = tableFromArrays({
    service: ["a", "b"],
    n: [1, 2],
  });
  const { columns, rows } = batchToRows(table.batches[0]);
  expect(columns.map((c) => c.name)).toEqual(["service", "n"]);
  expect(rows).toEqual([{ service: "a", n: 1 }, { service: "b", n: 2 }]);
});

test("autoChartAxes picks line for time + numeric", () => {
  const cols = [
    { name: "ts", kind: "time"    as const },
    { name: "n",  kind: "numeric" as const },
  ];
  expect(autoChartAxes(cols)).toEqual({ type: "line", x: "ts", y: "n" });
});

test("autoChartAxes picks bar for string + numeric", () => {
  const cols = [
    { name: "svc", kind: "string"  as const },
    { name: "n",   kind: "numeric" as const },
  ];
  expect(autoChartAxes(cols)).toEqual({ type: "bar", x: "svc", y: "n" });
});

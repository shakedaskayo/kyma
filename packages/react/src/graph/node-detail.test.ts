import { describe, expect, it } from "vitest";
import type { GraphNode } from "@kyma-ai/client";
import { nodeContent, nodeSourcePath, formatValue, orderedProps } from "./node-detail";

function node(properties: Record<string, unknown>, labels: string[] = ["Memory"]): GraphNode {
  return {
    id: "memory:abc",
    labels,
    properties,
    metadata: { created_at: "", updated_at: "", realm: "default" },
    namespace: "memory",
  };
}

describe("nodeContent", () => {
  it("returns a non-empty string content, else null", () => {
    expect(nodeContent(node({ content: "hello" }))).toBe("hello");
    expect(nodeContent(node({ content: "  " }))).toBeNull();
    expect(nodeContent(node({}))).toBeNull();
    expect(nodeContent(node({ content: 42 }))).toBeNull();
  });
});

describe("nodeSourcePath", () => {
  it("returns object_path when a non-empty string, else null", () => {
    expect(nodeSourcePath(node({ object_path: "artifacts/t/logs/x.log" }))).toBe("artifacts/t/logs/x.log");
    expect(nodeSourcePath(node({}))).toBeNull();
    expect(nodeSourcePath(node({ object_path: "" }))).toBeNull();
  });
});

describe("formatValue", () => {
  it("passes scalars through", () => {
    expect(formatValue("plain")).toEqual({ text: "plain", kind: "scalar" });
    expect(formatValue(0.7)).toEqual({ text: "0.7", kind: "scalar" });
    expect(formatValue(null)).toEqual({ text: "null", kind: "scalar" });
  });

  it("pretty-prints object values and JSON-string values", () => {
    expect(formatValue({ source: "claude" })).toEqual({
      text: '{\n  "source": "claude"\n}',
      kind: "json",
    });
    expect(formatValue('{"source":"claude"}')).toEqual({
      text: '{\n  "source": "claude"\n}',
      kind: "json",
    });
  });

  it("collapses large numeric arrays and embedding-like strings", () => {
    const arr = Array.from({ length: 40 }, (_, i) => i / 100);
    expect(formatValue(arr)).toEqual({ text: "float[40]", kind: "array" });
    const embStr = arr.map((n) => String(n)).join(",");
    expect(formatValue(embStr)).toEqual({ text: "float[40]", kind: "array" });
  });
});

describe("orderedProps", () => {
  it("puts content first, title second, embedding last, rest alphabetical", () => {
    const keys = orderedProps(
      node({ status: "active", embedding: "x", title: "T", content: "C", importance: 0.7 }),
    ).map(([k]) => k);
    expect(keys).toEqual(["content", "title", "importance", "status", "embedding"]);
  });
});

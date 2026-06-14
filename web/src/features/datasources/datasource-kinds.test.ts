import { describe, expect, it } from "vitest";
import type { CatalogEntry } from "@/sdk/datasources";
import { isWatcherDriven } from "./datasource-kinds";

const base: CatalogEntry = {
  type_id: "obsidian",
  label: "Obsidian",
  category: "knowledge",
  description: "",
  brand: "obsidian",
  auth_mode: "none",
  status: "available",
  drive_model: "continuous",
  default_schedule_ms: 60_000,
  fields: [],
};

describe("isWatcherDriven", () => {
  it("continuous entries skip the interval picker", () => {
    expect(isWatcherDriven({ ...base, drive_model: "continuous" })).toBe(true);
    expect(isWatcherDriven({ ...base, drive_model: "periodic" })).toBe(false);
  });
});

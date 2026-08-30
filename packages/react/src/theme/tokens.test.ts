import { describe, expect, it } from "vitest";
import { themeToCssVars } from "./tokens";
import { pensieveDark, pensieveLight } from "./presets";

describe("themeToCssVars", () => {
  it("maps camelCase tokens to --pensieve-kebab-case vars", () => {
    const vars = themeToCssVars({ background: "213 26% 7%", brandFrom: "180 72% 45%" });
    expect(vars["--pensieve-background"]).toBe("213 26% 7%");
    expect(vars["--pensieve-brand-from"]).toBe("180 72% 45%");
  });
  it("presets cover every token", () => {
    for (const preset of [pensieveDark, pensieveLight]) {
      const vars = themeToCssVars(preset);
      expect(Object.keys(vars).length).toBeGreaterThanOrEqual(26);
      expect(vars["--pensieve-font-sans"]).toBeTruthy();
    }
  });
});

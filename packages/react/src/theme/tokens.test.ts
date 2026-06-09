import { describe, expect, it } from "vitest";
import { themeToCssVars } from "./tokens";
import { kymaDark, kymaLight } from "./presets";

describe("themeToCssVars", () => {
  it("maps camelCase tokens to --kyma-kebab-case vars", () => {
    const vars = themeToCssVars({ background: "213 26% 7%", brandFrom: "180 72% 45%" });
    expect(vars["--kyma-background"]).toBe("213 26% 7%");
    expect(vars["--kyma-brand-from"]).toBe("180 72% 45%");
  });
  it("presets cover every token", () => {
    for (const preset of [kymaDark, kymaLight]) {
      const vars = themeToCssVars(preset);
      expect(Object.keys(vars).length).toBeGreaterThanOrEqual(26);
      expect(vars["--kyma-font-sans"]).toBeTruthy();
    }
  });
});

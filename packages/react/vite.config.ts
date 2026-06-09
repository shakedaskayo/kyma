import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import dts from "vite-plugin-dts";

export default defineConfig({
  plugins: [
    react(),
    // Don't emit declarations for tests, stories, or fixtures — they'd ship
    // to npm under dist/ otherwise (files: ["dist"]).
    dts({
      rollupTypes: false,
      exclude: ["**/*.test.*", "**/*.stories.*", "**/__fixtures__/**", ".storybook/**"],
    }),
  ],
  build: {
    lib: {
      entry: {
        index: "src/index.ts",
        graph: "src/graph/index.ts",
        query: "src/query/index.ts",
        discover: "src/discover/index.ts",
        dashboards: "src/dashboards/index.ts",
        agent: "src/agent/index.ts",
      },
      formats: ["es"],
    },
    sourcemap: true,
    rollupOptions: {
      external: (id) => !id.startsWith(".") && !id.startsWith("/") && !id.includes("?"),
    },
  },
  test: { environment: "jsdom" },
});

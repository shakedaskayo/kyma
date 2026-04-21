import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  fullyParallel: false,
  use: {
    baseURL: process.env.KYMA_WEB_URL ?? "http://localhost:8080",
    trace: "retain-on-failure",
  },
});

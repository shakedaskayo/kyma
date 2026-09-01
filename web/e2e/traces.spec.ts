import { test, expect, type Page } from "@playwright/test";

// Run against a live server with telemetry enabled:
//   PENSIEVE_WEB_URL=http://localhost:8080 \
//   PENSIEVE_E2E_TOKEN=<token> \
//   pnpm -C web e2e e2e/traces.spec.ts
//
// For local-mode (no token, PENSIEVE_AUTH_DISABLED=1):
//   HOME=$(mktemp -d) pensieve serve --addr 127.0.0.1:9091 &
//   PENSIEVE_WEB_URL=http://127.0.0.1:9091 pnpm -C web e2e e2e/traces.spec.ts

const TOKEN    = process.env.PENSIEVE_E2E_TOKEN ?? "";
const ENDPOINT = process.env.PENSIEVE_WEB_URL  ?? "http://localhost:8080";

/** Mirror the golden-path.spec.ts connect logic exactly. */
async function connect(page: Page) {
  await page.goto("/");
  await expect(page).toHaveURL(/\/settings/);

  await page.fill("input#endpoint", ENDPOINT);
  await page.fill("input#token",    TOKEN);
  await page.fill("input#database", "obs");
  await page.getByRole("button", { name: /save \+ connect/i }).click();

  await expect(page).toHaveURL(/\/explore/);
}

test("memory op → self-trace → visible on Traces page", async ({ page, request }) => {
  // 1. Cause a memory.recall span via the API.
  const res = await request.post(`${ENDPOINT}/v1/agent/memory/query`, {
    headers: { authorization: `Bearer ${TOKEN}` },
    data: { query: "traces e2e probe" },
  });
  expect(res.ok()).toBeTruthy();

  // 2. Connect the UI — mirror golden-path.spec.ts connect logic exactly.
  await connect(page);

  // 3. Traces page shows the request trace (batch exporter flushes ~5s).
  await page.goto("/traces");
  // Refresh until the row appears (batch export takes up to ~5s)
  await expect(async () => {
    await page.getByRole("button", { name: /refresh/i }).click();
    await expect(
      page.getByRole("cell", { name: /memory\.recall/ }).first(),
    ).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 30_000 });

  // 4. Drill in: waterfall shows the memory.recall span.
  await page.getByRole("cell", { name: /memory\.recall/ }).first().click();
  await expect(page.getByText("memory.recall").first()).toBeVisible({ timeout: 10_000 });
});

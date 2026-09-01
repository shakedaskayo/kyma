import { test, expect, type Page } from "@playwright/test";

// Walks every page of the web UI against a FRESH local-mode server
// (`pensieve serve` over an empty store — the exact post-install state).
// Every surface must render: working pages show their content/empty states,
// control-plane-only pages (data sources, credentials) show the capability
// gate — never a parse error, never a dead fetch.
//
// Run:
//   HOME=$(mktemp -d) pensieve serve --addr 127.0.0.1:9091 &
//   PENSIEVE_WEB_URL=http://127.0.0.1:9091 pnpm -C web e2e e2e/local-mode.spec.ts

const ENDPOINT = process.env.PENSIEVE_WEB_URL ?? "http://127.0.0.1:9091";

async function signIn(page: Page) {
  await page.goto("/login");
  await page.fill("input#endpoint", ENDPOINT);
  await page.fill("input#username", "admin");
  await page.fill("input#password", "admin");
  await page.getByRole("button", { name: /sign in/i }).click();
  await expect(page).toHaveURL(/\/(explore|discover|query)/, { timeout: 15_000 });
}

test("fresh local mode: every page renders, no fetch/parse errors", async ({ page }) => {
  const pageErrors: string[] = [];
  page.on("pageerror", (e) => pageErrors.push(String(e)));
  const failedJson: string[] = [];
  page.on("response", async (res) => {
    // Any API response the UI can't parse (HTML where JSON is expected)
    // would have been the old failure mode — assert it never happens.
    const url = res.url();
    if (url.includes("/v1/") && res.ok()) {
      const ct = res.headers()["content-type"] ?? "";
      if (ct.includes("text/html")) failedJson.push(url);
    }
  });

  await signIn(page);

  // Brand assets ship in the embedded bundle: the sidebar logo must actually
  // render (the server used to answer /icons/* with the SPA's HTML).
  const logo = page.locator('img[src="/icons/pensieve-mark.svg"]').first();
  await expect(logo).toBeVisible({ timeout: 10_000 });
  await expect
    .poll(async () => logo.evaluate((el: HTMLImageElement) => el.naturalWidth))
    .toBeGreaterThan(0);

  // Explore / Discover / Graph / Memory / Agent — supported locally.
  await page.goto("/discover");
  await expect(page.getByText(/discover|no data|sources/i).first()).toBeVisible({ timeout: 10_000 });

  await page.goto("/graph");
  await expect(page.locator("canvas").first()).toBeVisible({ timeout: 15_000 });

  await page.goto("/memory");
  await expect(page.getByText(/memor/i).first()).toBeVisible({ timeout: 10_000 });

  await page.goto("/agent");
  await expect(page.getByText(/agent|ask/i).first()).toBeVisible({ timeout: 10_000 });

  // Dashboards — read AND write work locally now.
  await page.goto("/dashboards");
  await expect(page.getByText(/dashboard/i).first()).toBeVisible({ timeout: 10_000 });

  // Data sources / Credentials — control-plane gates, not broken fetches.
  await page.goto("/data-sources");
  await expect(page.getByText(/control plane/i).first()).toBeVisible({ timeout: 10_000 });

  await page.goto("/credentials");
  await expect(page.getByText(/control plane/i).first()).toBeVisible({ timeout: 10_000 });

  // Settings — engine form renders (no 500 on unconfigured engines).
  await page.goto("/settings");
  await expect(page.getByText(/engine|provider|settings/i).first()).toBeVisible({ timeout: 10_000 });

  expect(failedJson, `HTML served on API paths: ${failedJson.join(", ")}`).toHaveLength(0);
  expect(pageErrors, `uncaught page errors: ${pageErrors.join("; ")}`).toHaveLength(0);
});

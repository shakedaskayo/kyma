import { test, expect, type Page } from "@playwright/test";

// Repro for: "i jump between pages on the ui and it persists the query editor
// page?? not moving between the views" — navigate between pages via the
// SIDEBAR LINKS (in-app SPA navigation, not full reloads) and assert the URL
// and content actually change.

const ENDPOINT = process.env.PENSIEVE_API_URL ?? "http://127.0.0.1:8080";

async function signIn(page: Page) {
  await page.goto("/login");
  await page.fill("input#endpoint", ENDPOINT);
  await page.fill("input#username", "admin");
  await page.fill("input#password", "admin");
  await page.getByRole("button", { name: /sign in/i }).click();
  await expect(page).toHaveURL(/\/(explore|discover|query)/, { timeout: 15_000 });
}

test("sidebar navigation actually moves between views", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    // React dev-mode advisories arrive as console.error with a "Warning:"
    // prefix and don't exist in prod builds — don't fail navigation on them.
    if (m.type() === "error" && !m.text().startsWith("Warning:")) errors.push(m.text());
  });

  await signIn(page);

  // Start at the Query Editor.
  await page.getByRole("link", { name: "Query Editor" }).click();
  await expect(page).toHaveURL(/\/query/, { timeout: 5_000 });

  // Now click through every sidebar destination and assert we ARRIVE and STAY.
  const hops: Array<[string, RegExp]> = [
    ["Discover", /\/discover/],
    ["Query Editor", /\/query/],
    ["Dashboards", /\/dashboards/],
    ["Graph", /\/graph/],
    ["Memory", /\/memory/],
    ["Agent", /\/agent/],
    ["Discover", /\/discover/],
  ];

  for (const [label, urlRe] of hops) {
    await page.getByRole("link", { name: label, exact: true }).click();
    await expect(page, `clicking "${label}" should navigate`).toHaveURL(urlRe, {
      timeout: 5_000,
    });
    // The killer assertion: 1.5s later we must STILL be there (no bounce-back).
    await page.waitForTimeout(1_500);
    await expect(page, `"${label}" must not bounce back`).toHaveURL(urlRe);
  }

  expect(errors, `console/page errors during navigation:\n${errors.join("\n")}`).toEqual([]);
});

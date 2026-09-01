import { test, expect } from "@playwright/test";

const TOKEN    = process.env.PENSIEVE_E2E_TOKEN ?? "";
const ENDPOINT = process.env.PENSIEVE_WEB_URL  ?? "http://localhost:8080";

test("settings → run KQL → see results → share URL → reload restores", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveURL(/\/settings/);

  await page.fill("input#endpoint", ENDPOINT);
  await page.fill("input#token",    TOKEN);
  await page.fill("input#database", "obs");
  await page.getByRole("button", { name: /save \+ connect/i }).click();

  await expect(page).toHaveURL(/\/explore/);

  // write a query, run it
  await page.locator(".monaco-editor").click();
  await page.keyboard.type("otel_logs | take 5");
  await page.keyboard.press("Meta+Enter");

  await expect(page.getByText(/rows ·/)).toBeVisible({ timeout: 15_000 });

  // URL encodes the state
  const url = page.url();
  expect(url).toMatch(/q=[A-Za-z0-9_-]+/);

  // reload → same state restored
  await page.goto(url);
  await expect(page.locator(".monaco-editor")).toContainText("take 5");
});

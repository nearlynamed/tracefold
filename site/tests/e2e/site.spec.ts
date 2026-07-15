import { expect, test } from "@playwright/test";

test("research results and provenance are navigable", async ({ page, request }, testInfo) => {
  const browserErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });
  page.on("pageerror", (error) => browserErrors.push(error.message));
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /Keep the answers/i })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Result explorer" })).toBeVisible();
  await expect(page.locator("[data-nextjs-dialog]")).toHaveCount(0);
  await expect(page.locator("body")).not.toHaveText("");
  await page.screenshot({ path: testInfo.outputPath("home.png"), fullPage: true });
  await page.getByRole("link", { name: "Data", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Data and provenance" })).toBeVisible();
  await expect(page.getByText(/sha256:/).first()).toBeVisible();
  const rawHref = await page.locator(".artifact a").first().getAttribute("href");
  expect(rawHref).toBeTruthy();
  const rawResponse = await request.get(rawHref!);
  expect(rawResponse.ok()).toBe(true);
  await page.getByRole("link", { name: "Paper", exact: true }).click();
  await expect(page.getByRole("heading", { name: /TraceFold: Query-Preserving/ })).toBeVisible();
  expect(browserErrors).toEqual([]);
});

import type { Page } from '@playwright/test';

/** Network settings with its folded groups (other networks, troubleshooting) open. */
export async function openNetworkSettings(page: Page) {
  await page.goto('/#/nodes');
  const groups = page.locator('details.cx-disclosure');
  await groups.first().waitFor();
  for (const group of await groups.all()) await group.evaluate(element => { (element as HTMLDetailsElement).open = true; });
}

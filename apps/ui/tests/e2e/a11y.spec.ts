import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

// Every route, mock backend, WCAG 2.0/2.1 A and AA. Zero violations is the bar (PLAN M5).
const ROUTES = [
  '/',
  '/channels',
  '/guide',
  '/movies',
  '/movies/6',
  '/series',
  '/series/4',
  '/recordings',
  '/search',
  '/settings/sources',
  '/settings/playback',
  '/settings/appearance',
  '/settings/updates',
  '/settings/about',
  '/player',
];

for (const route of ROUTES) {
  test(`axe: ${route}`, async ({ page }) => {
    await page.goto(route);
    await page.waitForLoadState('networkidle');
    if (route === '/guide') await expect(page.getByRole('gridcell').first()).toBeVisible();
    await page.waitForTimeout(600); // route fade-in + first data
    const { violations } = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']).analyze();
    expect(violations.map((v) => `${v.id} (${v.impact}) ${v.nodes[0]?.target.join(' ')}`)).toEqual([]);
  });
}

test('axe: command palette open', async ({ page }) => {
  await page.goto('/');
  await page.waitForLoadState('networkidle');
  await page.keyboard.press('Control+K');
  await page.keyboard.type('harb');
  await page.waitForTimeout(500);
  const { violations } = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(violations.map((v) => `${v.id} ${v.nodes[0]?.target.join(' ')}`)).toEqual([]);
});

import { expect, test, type Page } from '@playwright/test';

// M5b flows against the mock backend: track/stream menus, mini player, multiview.

const calls = (page: Page) => page.evaluate(() => (window as unknown as { __mockCalls?: string[] }).__mockCalls ?? []);
const settle = (page: Page) => page.waitForLoadState('networkidle');

async function playFirstChannel(page: Page) {
  await page.goto('/channels');
  await settle(page);
  await page.locator('[role=list] [data-index="0"]').focus();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/player$/);
  await expect(page.getByRole('button', { name: 'Audio (A)' })).toBeVisible();
  await page.waitForTimeout(250);
}

test('player: audio, subtitle and stream menus switch tracks and streams', async ({ page }) => {
  await playFirstChannel(page);
  await page.keyboard.press('a');
  await page.getByRole('menuitem', { name: /DEU/ }).click();
  await page.keyboard.press('a');
  await expect(page.getByRole('menuitem', { name: /DEU/ })).toHaveAttribute('aria-current', 'true');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toHaveCount(0);
  await page.keyboard.press('t');
  await page.getByRole('menuitem', { name: /Teletext 888/ }).click();
  await page.getByRole('button', { name: 'Stream' }).click();
  await expect(page.getByText('Switching changes the stream for everyone watching this channel.')).toBeVisible();
  await page.getByRole('menuitem', { name: /Backup A/ }).click();
  await expect.poll(async () => (await calls(page)).includes('switch_stream')).toBe(true);
});

test('mini player: P enters and leaves compact mode, the rail hides', async ({ page }) => {
  await playFirstChannel(page);
  await page.keyboard.press('p');
  await expect(page.locator('[data-mini]')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Restore window (P)' })).toBeAttached();
  await page.keyboard.press('p');
  await expect(page.locator('[data-mini]')).toHaveCount(0);
  const n = (await calls(page)).filter((c) => c === 'set_mini_player').length;
  expect(n).toBe(2);
});

test('multiview: add a tile from the picker without any play while browsing, listen, exit stops tiles', async ({ page }) => {
  await playFirstChannel(page);
  await page.getByRole('button', { name: 'Multiview' }).click();
  await expect(page).toHaveURL(/\/multiview$/);
  const plays = async () => (await calls(page)).filter((c) => c.startsWith('play_') || c === 'multiview_set_tile').length;
  const tiles = page.locator('[role=group][aria-label^="Slot"]');
  await expect(tiles).toHaveCount(1); // one channel fills the view, no empty slots to pick a grid for
  const before = await plays();
  await page.getByRole('button', { name: 'Add channel' }).click();
  await page.getByLabel('Search channels').fill('Cobalt');
  await page.waitForTimeout(400);
  expect(await plays()).toBe(before); // browsing the picker is SQLite only (rule 4)
  await page.getByRole('option', { name: /Cobalt Sports/ }).first().click();
  await expect(page.getByRole('group', { name: /Slot 2: Cobalt Sports/ })).toBeVisible();
  await expect(page.getByText('2 of 4 streams')).toBeVisible();
  await expect(tiles).toHaveCount(2); // reflowed side by side on its own
  await expect(page.getByRole('radiogroup', { name: 'Layout (L)' })).toHaveCount(0); // two tiles have one layout
  await page.keyboard.press('2');
  await expect.poll(async () => (await calls(page)).includes('multiview_set_audio')).toBe(true);
  await page.keyboard.press('Escape');
  await expect(page).toHaveURL(/\/player$/);
  await expect.poll(async () => (await calls(page)).includes('multiview_exit')).toBe(true);
});

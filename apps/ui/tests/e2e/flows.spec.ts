import { expect, test, type Page } from '@playwright/test';

// PLAN §11.4 item 5: end-to-end flows against the in-browser mock backend. Fast and deterministic:
// no network, no real time except the short waits for the route fade and the mock's own timers.

const calls = (page: Page) => page.evaluate(() => (window as unknown as { __mockCalls?: string[] }).__mockCalls ?? []);
const settle = (page: Page) => page.waitForLoadState('networkidle');
const focusFirstRow = (page: Page) => page.locator('[role=list] [data-index="0"]').focus();

test('first run: add a Dispatcharr source, test it, save, land on Channels with the first row focused', async ({ page }) => {
  await page.goto('/?mock=empty');
  await page.getByRole('button', { name: 'Add source' }).click();
  await page.getByLabel('Server URL').fill('http://192.0.2.10:9191');
  await page.getByLabel('API key').fill('example-key');
  await page.getByRole('button', { name: 'Test connection' }).click();
  await expect(page.getByText(/channels · .* programmes/)).toBeVisible();
  await page.getByRole('button', { name: 'Save and open Live TV' }).click();
  await expect(page).toHaveURL(/\/channels$/);
  await expect(page.locator('[role=list] [data-index="0"]')).toBeFocused();
});

test('channels keyboard: down down Enter plays, zap pill, number entry, Esc goes back', async ({ page }) => {
  await page.goto('/channels');
  await settle(page);
  await focusFirstRow(page);
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/player$/);
  await expect(page.locator('[data-video-rect]')).toBeVisible(); // lazy player chunk mounted
  await expect(page.getByText('Cobalt Sports').first()).toBeVisible(); // third channel
  await page.waitForTimeout(250); // key listeners attach in an effect after mount
  // "5" has no exact match: the lowest number >= 5 wins (channel 5.5, PLAN §8.4). Enter confirms early.
  await page.keyboard.press('5');
  await expect(page.getByRole('status', { name: 'Channel number' })).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(page.getByText('Orchard Kids').first()).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page).toHaveURL(/\/channels$/);
});

test('guide: Home goes to now, a past+archive block opens catch-up with a seek bar, End goes live; dwell never plays', async ({ page }) => {
  await page.goto('/guide');
  await settle(page);
  await expect(page.getByRole('gridcell').first()).toBeVisible(); // lazy chunk + first data
  const grid = page.getByRole('grid').first();
  await grid.focus();
  await page.keyboard.press('Home');
  for (let i = 0; i < 12 && !(await page.locator('[data-focused][data-state="past-archive"]').count()); i++) {
    await page.keyboard.press('ArrowLeft');
  }
  await expect(page.locator('[data-focused][data-state="past-archive"]')).toBeVisible();
  await page.waitForTimeout(700); // longer than the 300 ms preview dwell
  expect((await calls(page)).filter((c) => c.startsWith('play_') || c === 'restart_programme')).toEqual([]);
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/player$/);
  await expect(page.getByRole('slider', { name: 'Seek' })).toBeVisible();
  await page.keyboard.press('End');
  await expect(page.getByRole('slider', { name: 'Seek' })).toHaveCount(0);
  await expect(page.getByText('LIVE').first()).toBeVisible();
});

test('search: / opens search, typing lists results, Enter plays the first', async ({ page }) => {
  await page.goto('/');
  await settle(page);
  await page.keyboard.press('/');
  await expect(page).toHaveURL(/\/search$/);
  await page.getByRole('combobox').fill('harbor');
  await expect(page.getByRole('option').first()).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/player$/);
});

test('favourites: F toggles the focused channel and the sidebar count follows', async ({ page }) => {
  await page.goto('/channels');
  await settle(page);
  const fav = page.locator('nav[aria-label="Groups"] button', { hasText: 'Favourites' });
  const before = Number((await fav.innerText()).replace(/\D/g, ''));
  await focusFirstRow(page);
  await page.keyboard.press('f');
  await expect.poll(async () => Number((await fav.innerText()).replace(/\D/g, ''))).not.toBe(before);
});

test('10-foot: ?tv sets tv density and arrows move the selection', async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto('/channels?tv');
  await settle(page);
  await expect(page.locator('html')).toHaveAttribute('data-density', 'tv');
  await focusFirstRow(page);
  await page.keyboard.press('ArrowDown');
  await expect(page.locator('[role=list] [data-index="1"]')).toHaveAttribute('aria-current', 'true');
});

test('offline source shows the banner with a Retry action', async ({ page }) => {
  await page.goto('/?mock=offline');
  await settle(page);
  await expect(page.getByText(/Offline — last updated/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retry' })).toBeVisible();
});

test('a channel that fails shows a plain message and Retry, never a hang', async ({ page }) => {
  await page.goto('/channels?fail=102');
  await settle(page);
  await focusFirstRow(page);
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Try again' })).toBeVisible();
});

test('a failed play command raises one toast with the error message', async ({ page }) => {
  await page.goto('/channels?failplay=1');
  await settle(page);
  await focusFirstRow(page);
  await page.keyboard.press('Enter');
  const toast = page.getByRole('alert').filter({ hasText: 'The server did not answer' });
  await expect(toast).toHaveCount(1);
  await expect(page).toHaveURL(/\/channels\?/);
});

test('Ctrl+K opens the command palette from anywhere, even right after load and from a text field', async ({ page }) => {
  await page.goto('/channels');
  await page.keyboard.press('Control+K'); // may precede the idle mount: the request is queued
  const box = page.getByRole('textbox', { name: 'Command palette' });
  await expect(box).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(box).toBeHidden();
  await page.getByRole('textbox', { name: 'Type to filter' }).focus();
  await page.keyboard.press('Control+K');
  await expect(box).toBeVisible();
  await page.keyboard.type('harb');
  await expect(page.getByText('Harbor News 24').first()).toBeVisible();
});

test('player: F toggles favourite with a confirmation, F11 and double-click toggle full screen', async ({ page }) => {
  await page.goto('/channels');
  await settle(page);
  await focusFirstRow(page);
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/player$/);
  await expect(page.locator('[data-video-rect]')).toBeVisible();
  await page.waitForTimeout(250);
  await page.keyboard.press('f');
  await expect(page.getByText(/favourites\.$/)).toBeVisible();
  await expect(page.locator('html')).not.toHaveAttribute('data-fullscreen', 'true');
  const calls0 = (await calls(page)).filter((c) => c === 'set_fullscreen').length;
  await page.keyboard.press('F11');
  await page.locator('[data-video-rect]').dblclick({ position: { x: 300, y: 300 } });
  await expect.poll(async () => (await calls(page)).filter((c) => c === 'set_fullscreen').length).toBe(calls0 + 2);
});

test('offline: record actions are disabled with the reason (guide menu, recordings page)', async ({ page }) => {
  await page.goto('/recordings?mock=offline');
  await settle(page);
  await expect(page.getByText('Recording needs a connection to your server.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Cancel' }).first()).toBeDisabled();
  await page.goto('/guide?mock=offline');
  await settle(page);
  await expect(page.getByRole('gridcell').first()).toBeVisible();
  await page.getByRole('grid').first().focus();
  await page.keyboard.press('ArrowRight'); // a now/future block
  await page.keyboard.press('r');
  await expect(page.getByText('Recording needs a connection to your server.')).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Record this programme' })).toBeDisabled();
});

for (const c of [
  { reason: 'limit', title: 'Your server is at its stream limit', first: 'Try again' },
  { reason: 'codec', title: 'This channel uses a format Downlink can’t decode', body: 'AV1 10-bit', first: 'Copy diagnostics' },
  { reason: 'engine', title: 'The video engine stopped unexpectedly', body: 'c-7f3a', first: 'Copy diagnostics' },
]) {
  test(`player failure copy: ${c.reason}`, async ({ page }) => {
    await page.goto(`/channels?fail=102&reason=${c.reason}`);
    await settle(page);
    await focusFirstRow(page);
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('ArrowDown');
    await page.keyboard.press('Enter');
    const alert = page.getByRole('alert').filter({ hasText: c.title });
    await expect(alert).toBeVisible();
    if (c.body) await expect(alert).toContainText(c.body);
    await expect(alert.getByRole('button').first()).toHaveText(c.first);
  });
}

test('player shows "Reconnecting…" during an automatic retry', async ({ page }) => {
  await page.goto('/channels?reconnect=1');
  await settle(page);
  await focusFirstRow(page);
  await page.keyboard.press('Enter');
  await expect(page.getByRole('status').filter({ hasText: 'Reconnecting…' })).toBeVisible();
});

test('blocking prompt: a newer database stops the app; "Start fresh" is outlined, not a filled primary', async ({ page }) => {
  await page.goto('/channels?mock=newerdb');
  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toContainText('This library was created by a newer version of Downlink.');
  await expect(dialog.getByRole('button', { name: 'Quit' })).toHaveAttribute('data-variant', 'filled');
  await expect(dialog.getByRole('button', { name: 'Start fresh' })).toHaveAttribute('data-variant', 'danger');
  const reloaded = page.waitForEvent('load'); // after archiving, the UI reloads
  await dialog.getByRole('button', { name: 'Start fresh' }).click();
  await reloaded;
});

test('blocking prompt: a changed certificate offers Trust new certificate or Cancel', async ({ page }) => {
  await page.goto('/channels?mock=cert');
  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toContainText('certificate changed');
  await dialog.getByRole('button', { name: 'Trust new certificate' }).click();
  await expect(dialog).toBeHidden();
  await expect.poll(async () => (await calls(page)).includes('trust_new_certificate')).toBe(true);
  await expect(page.locator('[role=list] [data-index="0"]')).toBeVisible();
});

test('settings: the debug logging switch is in About & diagnostics and calls set_debug_logging', async ({ page }) => {
  await page.goto('/settings/about');
  await settle(page);
  await page.getByLabel('Debug logging').click({ force: true });
  await expect.poll(async () => (await calls(page)).includes('set_debug_logging')).toBe(true);
});

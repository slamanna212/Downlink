import { expect, test } from '@playwright/test';

// The guide's now pill and block times follow the browser time zone, derived from UTC rows.
const NOW = '2026-11-19T12:00:00Z';
for (const { tz, clock } of [
  { tz: 'America/New_York', clock: '07:00' }, // EST, UTC-5
  { tz: 'Pacific/Chatham', clock: '01:45' }, // +13:45 in November
]) {
  test.describe(tz, () => {
    test.use({ timezoneId: tz });
    test(`guide shows local time ${clock} on the now pill`, async ({ page }) => {
      await page.clock.setFixedTime(new Date(NOW));
      await page.goto('/guide');
      await page.waitForLoadState('networkidle');
      await expect(page.locator('[class*="nowPill"]')).toHaveText(clock);
      // every tick label is on a local :00 or :30 even in a +13:45 zone
      const labels = await page.locator('[class*="tick"]').allInnerTexts();
      expect(labels.length).toBeGreaterThan(3);
      for (const l of labels.filter((x) => /^\d\d:\d\d$/.test(x))) expect(l).toMatch(/:(00|30)$/);
    });
  });
}

import { expect, test, type Page } from '@playwright/test';
import { backdrop, poster, videoFrame } from './art';

// Marketing screenshots: `pnpm screenshots` writes PNGs to apps/ui/screenshots/. Production build + mock backend
// in showcase mode (synthetic data, procedural art). Nothing here touches the network.

const OUT = 'screenshots';
const svgUrl = (svg: string) => `data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}`;

test.beforeEach(async ({ page }) => {
  await page.route('**/showcase/**', (route) => {
    const [, kind, n] = new URL(route.request().url()).pathname.match(/\/showcase\/(poster|backdrop)\/(\d+)/) ?? [];
    if (!kind) return route.fulfill({ status: 404 });
    return route.fulfill({ contentType: 'image/svg+xml', body: (kind === 'poster' ? poster : backdrop)(Number(n)) });
  });
});

async function open(page: Page, path: string) {
  await page.goto(`${path}${path.includes('?') ? '&' : '?'}mock=showcase`);
  await page.waitForLoadState('networkidle');
  // Dev-only stand-ins for the libmpv surface; the real app has video there.
  await page.addStyleTag({ content: '[class*="_devVideo_"], [class*="_miniDev_"] { display: none !important; }' });
}

/** Paints a fake video frame under each transparent rect matching `selector` (where libmpv draws in the real app). */
async function fakeVideo(page: Page, selector: string, frames: string[] = [videoFrame()]) {
  const boxes = await page.locator(selector).evaluateAll((els) => els.map((el) => {
    const r = el.getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  }));
  if (boxes.length === 0) throw new Error(`no video rect for ${selector}`);
  await page.evaluate(({ boxes, srcs }) => {
    document.querySelectorAll('[data-shot-video]').forEach((el) => el.remove());
    boxes.forEach((box, i) => {
      const img = document.createElement('img');
      img.dataset.shotVideo = '';
      img.src = srcs[i % srcs.length]!;
      Object.assign(img.style, {
        position: 'fixed', left: `${box.x}px`, top: `${box.y}px`, width: `${box.width}px`, height: `${box.height}px`,
        objectFit: 'cover', zIndex: '-1', pointerEvents: 'none',
      });
      document.body.prepend(img);
    });
  }, { boxes, srcs: frames.map(svgUrl) });
  await page.waitForTimeout(100);
}

async function shoot(page: Page, name: string) {
  await page.waitForTimeout(600); // route fade-in and lazy images
  await page.screenshot({ path: `${OUT}/${name}.png` });
}

test('home', async ({ page }) => {
  await open(page, '/');
  await shoot(page, 'home');
});

test('guide with video playing', async ({ page }) => {
  await open(page, '/guide');
  await page.getByRole('button', { name: 'Watch live' }).click();
  await expect(page).toHaveURL(/\/player/);
  await page.waitForTimeout(500);
  await page.keyboard.press('Escape');
  await expect(page).toHaveURL(/\/guide/);
  await page.mouse.move(1200, 900); // off the preview so no hover state
  await page.waitForTimeout(500);
  await fakeVideo(page, `button[aria-label="Open player"]`);
  await shoot(page, 'guide-playing');
});

test('movies browse', async ({ page }) => {
  await open(page, '/movies');
  await shoot(page, 'movies');
});

test('now playing', async ({ page }) => {
  await open(page, '/channels');
  await page.locator('[role=list] [data-index="8"]').focus();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/player/);
  await expect(page.locator('[data-video-rect]')).toBeVisible();
  await page.waitForTimeout(800);
  await fakeVideo(page, '[data-video-rect]');
  await page.mouse.move(900, 600);
  await page.mouse.move(910, 610); // wake the control bar
  await shoot(page, 'now-playing');
});

test('multiview, one large and two stacked', async ({ page }) => {
  await open(page, '/channels');
  await page.locator('[role=list] [data-index="8"]').focus(); // Tidepool Nature
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/player/);
  await page.getByRole('button', { name: 'Multiview' }).click();
  await expect(page).toHaveURL(/\/multiview/);
  const tiles = page.locator('[role=group][aria-label^="Slot"]');
  await expect(tiles).toHaveCount(1);
  // Same flow as tests/e2e/multiview.spec.ts: one picker, pick both, then its Add channel button closes it.
  await page.getByRole('button', { name: 'Add channel' }).click();
  for (const name of ['Cobalt Sports', 'Summit Music']) {
    await page.getByLabel('Search channels').fill(name);
    await page.waitForTimeout(300);
    await page.getByRole('option', { name: new RegExp(name) }).first().click();
  }
  await page.getByRole('dialog', { name: /Add channel/ }).waitFor();
  await page.getByRole('button', { name: /Add channel/ }).click();
  await expect(tiles).toHaveCount(3);
  await page.getByRole('radiogroup', { name: 'Layout (L)' }).getByText('One large, others small').click({ force: true });
  // Wake the toolbar, then rest on its edge so no tile shows its hover controls.
  const bar = (await page.getByRole('button', { name: /Add channel/ }).boundingBox())!;
  await page.mouse.move(bar.x - 4, bar.y + bar.height / 2 - 20);
  await page.mouse.move(bar.x - 4, bar.y + bar.height / 2);
  await page.waitForTimeout(500); // grid reflow
  await fakeVideo(page, '[role=group][aria-label^="Slot"]', [videoFrame(2), videoFrame(3), videoFrame(1)]);
  await shoot(page, 'multiview');
});

test('recordings', async ({ page }) => {
  await open(page, '/recordings');
  await shoot(page, 'recordings');
});

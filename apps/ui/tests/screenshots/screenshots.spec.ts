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

/** Paints a fake video frame under the transparent rect `selector` (where libmpv draws in the real app). */
async function fakeVideo(page: Page, selector: string, frame = videoFrame()) {
  const box = await page.locator(selector).first().boundingBox();
  if (!box) throw new Error(`no video rect for ${selector}`);
  await page.evaluate(({ box, src }) => {
    document.getElementById('shot-video')?.remove();
    const img = document.createElement('img');
    img.id = 'shot-video';
    img.src = src;
    Object.assign(img.style, {
      position: 'fixed', left: `${box.x}px`, top: `${box.y}px`, width: `${box.width}px`, height: `${box.height}px`,
      objectFit: 'cover', zIndex: '-1', pointerEvents: 'none',
    });
    document.body.prepend(img);
  }, { box, src: svgUrl(frame) });
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

test('recordings', async ({ page }) => {
  await open(page, '/recordings');
  await shoot(page, 'recordings');
});

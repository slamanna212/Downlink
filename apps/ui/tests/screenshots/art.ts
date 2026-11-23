// Procedural stand-in artwork for marketing screenshots: layered-landscape SVGs, deterministic per index.
// Synthetic only (CLAUDE rule 16). Real art can replace it: see tests/screenshots/README.md.

interface Palette {
  sky: [string, string];
  sun: string;
  hills: [string, string, string];
  stars?: boolean;
}

const PALETTES: Palette[] = [
  { sky: ['rgb(255,138,101)', 'rgb(255,214,153)'], sun: 'rgb(255,244,214)', hills: ['rgb(196,86,92)', 'rgb(120,52,84)', 'rgb(52,24,56)'] },
  { sky: ['rgb(16,24,64)', 'rgb(72,58,140)'], sun: 'rgb(236,232,255)', hills: ['rgb(58,52,120)', 'rgb(34,30,82)', 'rgb(14,12,40)'], stars: true },
  { sky: ['rgb(120,200,230)', 'rgb(220,244,250)'], sun: 'rgb(255,255,240)', hills: ['rgb(96,170,140)', 'rgb(44,120,104)', 'rgb(18,62,60)'] },
  { sky: ['rgb(255,94,98)', 'rgb(255,190,118)'], sun: 'rgb(255,236,180)', hills: ['rgb(150,60,90)', 'rgb(88,34,74)', 'rgb(30,14,40)'] },
  { sky: ['rgb(30,60,90)', 'rgb(110,170,190)'], sun: 'rgb(230,250,255)', hills: ['rgb(60,110,130)', 'rgb(32,70,92)', 'rgb(12,32,48)'] },
  { sky: ['rgb(60,20,80)', 'rgb(220,90,120)'], sun: 'rgb(255,210,170)', hills: ['rgb(140,50,100)', 'rgb(80,28,72)', 'rgb(28,10,36)'], stars: true },
  { sky: ['rgb(250,200,120)', 'rgb(255,240,200)'], sun: 'rgb(255,255,235)', hills: ['rgb(200,140,80)', 'rgb(140,84,52)', 'rgb(70,40,30)'] },
  { sky: ['rgb(8,12,30)', 'rgb(30,70,110)'], sun: 'rgb(210,240,255)', hills: ['rgb(30,80,100)', 'rgb(16,46,66)', 'rgb(6,18,30)'], stars: true },
];

/** Small deterministic PRNG (mulberry32) so every render of index n is identical. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function ridge(r: () => number, w: number, h: number, base: number, amp: number, steps: number): string {
  const pts: string[] = [`0,${h}`];
  let y = base;
  for (let i = 0; i <= steps; i++) {
    y = Math.max(base - amp, Math.min(base + amp * 0.4, y + (r() - 0.55) * amp * 0.6));
    pts.push(`${((i / steps) * w).toFixed(1)},${y.toFixed(1)}`);
  }
  pts.push(`${w},${h}`);
  return pts.join(' ');
}

function scene(n: number, w: number, h: number): string {
  const p = PALETTES[n % PALETTES.length]!;
  const r = rng(n * 7919 + w);
  const sunX = w * (0.25 + r() * 0.5);
  const sunY = h * (0.28 + r() * 0.18);
  const sunR = Math.min(w, h) * (0.08 + r() * 0.06);
  const stars = p.stars
    ? Array.from({ length: 70 }, () => `<circle cx="${(r() * w).toFixed(1)}" cy="${(r() * h * 0.5).toFixed(1)}" r="${(0.6 + r() * 1.4).toFixed(2)}" fill="white" opacity="${(0.3 + r() * 0.6).toFixed(2)}"/>`).join('')
    : '';
  const layers = p.hills
    .map((c, i) => `<polygon points="${ridge(r, w, h, h * (0.55 + i * 0.13), h * (0.16 - i * 0.03), 10 + i * 6)}" fill="${c}"/>`)
    .join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">
<defs>
<linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${p.sky[0]}"/><stop offset="1" stop-color="${p.sky[1]}"/></linearGradient>
<radialGradient id="g"><stop offset="0" stop-color="${p.sun}" stop-opacity="0.9"/><stop offset="1" stop-color="${p.sun}" stop-opacity="0"/></radialGradient>
<radialGradient id="v" cx="0.5" cy="0.45" r="0.75"><stop offset="0.6" stop-color="black" stop-opacity="0"/><stop offset="1" stop-color="black" stop-opacity="0.45"/></radialGradient>
</defs>
<rect width="${w}" height="${h}" fill="url(#s)"/>${stars}
<circle cx="${sunX}" cy="${sunY}" r="${sunR * 3}" fill="url(#g)"/>
<circle cx="${sunX}" cy="${sunY}" r="${sunR}" fill="${p.sun}"/>
${layers}
<rect width="${w}" height="${h}" fill="url(#v)"/>
</svg>`;
}

export const poster = (n: number): string => scene(n, 600, 900);
export const backdrop = (n: number): string => scene(n + 3, 1920, 1080);
/** The "video" shown under the transparent player surface. */
export const videoFrame = (n = 2): string => scene(n, 1920, 1080);

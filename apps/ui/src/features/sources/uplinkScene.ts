// The "Uplink" first-sync scene (DESIGN §12): the app icon brought to life on a 2D canvas. A ground
// station on a cratered planet tracks a satellite whose orbit fills in as the sync progresses. Colours are
// read from the theme's CSS variables, never written here (DESIGN §11 rule 1).
import type { SyncPhase } from './syncProgress';

export interface UplinkPalette {
  sky: string;
  night: string;
  accent: string;
  accentHover: string;
  accentPressed: string;
  text: string;
  cyan: string;
  blue: string;
  mount: string;
  crater: string;
}

const VARS: Record<keyof UplinkPalette, string> = {
  sky: '--dl-surface-0',
  night: '--dl-uplink-night',
  accent: '--dl-accent',
  accentHover: '--dl-accent-hover',
  accentPressed: '--dl-accent-pressed',
  text: '--dl-text',
  cyan: '--dl-sparkle-cyan',
  blue: '--dl-sparkle-blue',
  mount: '--dl-uplink-mount',
  crater: '--dl-uplink-crater',
};

export function readPalette(el: Element): UplinkPalette {
  const css = getComputedStyle(el);
  const out = {} as UplinkPalette;
  for (const k of Object.keys(VARS) as (keyof UplinkPalette)[]) out[k] = css.getPropertyValue(VARS[k]).trim();
  return out;
}

/** `#rrggbb` + alpha → rgba(). Anything else (a missing variable) draws nothing. */
export function withAlpha(hex: string, a: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) return 'rgba(0,0,0,0)';
  const n = parseInt(m[1]!, 16);
  return `rgba(${n >> 16},${(n >> 8) & 255},${n & 255},${a})`;
}

const TAU = Math.PI * 2;
const clamp = (x: number, a = 0, b = 1) => Math.max(a, Math.min(b, x));

/** Deterministic layout: the same sky every time. */
function seeded(seed: number) {
  let s = seed;
  return () => (s = (s * 16807) % 2147483647) / 2147483647;
}

type Colour = keyof UplinkPalette;
interface Star { x: number; y: number; sparkle: boolean; big: boolean; p: number; sp: number; r: number; c: Colour; rot: number }
interface Crater { u: number; rho: number; r: number }
interface Dust { u: number; rho: number; a: number; light: boolean }
interface Band { rho: number; w: number; light: boolean }

function layout() {
  const rand = seeded(42);
  const dots: Colour[] = ['text', 'accentHover', 'cyan', 'blue'];
  const stars: Star[] = [];
  for (let i = 0; i < 170; i++) {
    const sparkle = rand() < 0.11;
    const big = rand() < 0.15;
    stars.push({
      x: rand(),
      y: rand() * 0.72,
      sparkle,
      big,
      p: rand() * TAU,
      sp: 0.4 + rand() * 1.4,
      r: sparkle ? (big ? 7 + rand() * 5 : 3 + rand() * 3) : big ? 1.7 + rand() * 1.1 : 0.5 + rand() * 0.9,
      c: sparkle ? (rand() < 0.5 ? 'cyan' : 'blue') : dots[Math.floor(rand() * 4)]!,
      rot: sparkle && rand() < 0.5 ? (rand() - 0.5) * 0.3 : 0,
    });
  }
  const craters: Crater[] = [];
  while (craters.length < 26) {
    const u = rand() * 2 - 1;
    const rho = 0.78 + rand() * 0.215;
    if (Math.abs(u) < 0.1 && rho > 0.94) continue; // keep the ground station's footing clear
    craters.push({ u, rho, r: 0.012 + Math.pow(rand(), 2.2) * 0.05 });
  }
  const dust: Dust[] = Array.from({ length: 220 }, () => ({ u: rand() * 2 - 1, rho: 0.76 + rand() * 0.235, a: 0.15 + rand() * 0.35, light: rand() < 0.5 }));
  const bands: Band[] = [0.985, 0.955, 0.93, 0.885, 0.84, 0.8].map((rho, i) => ({ rho, w: 0.004 + rand() * 0.01, light: i % 2 === 0 }));
  return { stars, craters, dust, bands };
}

export interface UplinkFrame {
  phase: SyncPhase;
  /** 0..1 target; the scene eases toward it. */
  overall: number;
}

export interface UplinkScene {
  /** `t` = seconds of scene time (frozen under reduced motion), `dt` = seconds since the last frame. */
  draw(ctx: CanvasRenderingContext2D, w: number, h: number, t: number, dt: number, frame: UplinkFrame): void;
}

export function createUplinkScene(col: UplinkPalette, reduced: boolean): UplinkScene {
  const { stars, craters, dust, bands } = layout();
  let shown = 0;
  let dishAng = 0;
  let ledFor = 0;
  let doneFor = 0;

  const rrect = (ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) => {
    ctx.beginPath();
    if (ctx.roundRect) ctx.roundRect(x, y, w, h, r);
    else ctx.rect(x, y, w, h);
  };
  const sparkle = (ctx: CanvasRenderingContext2D, x: number, y: number, r: number, fill: string, rot: number) => {
    ctx.save();
    ctx.translate(x, y);
    ctx.rotate(rot);
    ctx.fillStyle = fill;
    ctx.strokeStyle = fill;
    ctx.lineWidth = r * 0.14;
    ctx.lineJoin = 'round';
    ctx.beginPath();
    ctx.moveTo(0, -r);
    ctx.quadraticCurveTo(0, 0, r, 0);
    ctx.quadraticCurveTo(0, 0, 0, r);
    ctx.quadraticCurveTo(0, 0, -r, 0);
    ctx.quadraticCurveTo(0, 0, 0, -r);
    ctx.fill();
    ctx.stroke();
    ctx.restore();
  };
  const satellite = (ctx: CanvasRenderingContext2D, x: number, y: number, u: number, rot: number) => {
    ctx.save();
    ctx.translate(x, y);
    ctx.rotate(rot);
    ctx.fillStyle = col.accentPressed;
    ctx.fillRect(-u * 2.6, -u * 0.45, u * 1.7, u * 0.9);
    ctx.fillRect(u * 0.9, -u * 0.45, u * 1.7, u * 0.9);
    ctx.strokeStyle = withAlpha(col.night, 0.6);
    ctx.lineWidth = 1;
    for (let i = 1; i < 3; i++) {
      for (const x0 of [-u * 2.6, u * 0.9]) {
        ctx.beginPath();
        ctx.moveTo(x0 + i * u * 0.57, -u * 0.45);
        ctx.lineTo(x0 + i * u * 0.57, u * 0.45);
        ctx.stroke();
      }
    }
    ctx.fillStyle = col.text;
    ctx.fillRect(-u * 0.9, -u * 0.1, u * 1.8, u * 0.2);
    ctx.fillStyle = col.accentHover;
    rrect(ctx, -u * 0.7, -u * 0.7, u * 1.4, u * 1.4, u * 0.25);
    ctx.fill();
    ctx.fillStyle = col.text;
    ctx.beginPath();
    ctx.arc(0, u * 0.95, u * 0.22, 0, TAU);
    ctx.fill();
    ctx.restore();
  };

  return {
    draw(ctx, w, h, t, dt, frame) {
      const done = frame.phase === 'done';
      shown = reduced ? frame.overall : shown + (frame.overall - shown) * (1 - Math.exp(-dt * 3));
      doneFor = done ? doneFor + dt : 0;
      ledFor = Math.max(0, ledFor - dt);

      // sky
      const g = ctx.createLinearGradient(0, 0, 0, h);
      g.addColorStop(0, col.sky);
      g.addColorStop(0.75, col.night);
      g.addColorStop(1, col.night);
      ctx.fillStyle = g;
      ctx.fillRect(0, 0, w, h);
      const scale = clamp(h / 300, 0.6, 1.4);
      for (const s of stars) {
        const tw = 0.5 + 0.5 * Math.sin(t * s.sp + s.p);
        const x = s.x * w;
        const y = s.y * h;
        if (s.sparkle) {
          const r = s.r * scale * (0.82 + 0.18 * tw);
          if (s.r > 7) {
            const hg = ctx.createRadialGradient(x, y, 0, x, y, r * 2.4);
            hg.addColorStop(0, withAlpha(col[s.c], 0.22 * tw));
            hg.addColorStop(1, withAlpha(col[s.c], 0));
            ctx.fillStyle = hg;
            ctx.beginPath();
            ctx.arc(x, y, r * 2.4, 0, TAU);
            ctx.fill();
          }
          sparkle(ctx, x, y, r, withAlpha(col[s.c], 0.65 + 0.35 * tw), s.rot * t);
        } else {
          ctx.fillStyle = withAlpha(col[s.c], (s.big ? 0.55 : 0.3) + 0.45 * tw);
          ctx.beginPath();
          ctx.arc(x, y, s.r * scale, 0, TAU);
          ctx.fill();
        }
      }

      // planet
      const horizon = h * 0.7;
      const R = Math.max(w, h) * 1.15;
      const cx = w * 0.5;
      const cy = horizon + R;
      const span = Math.asin(Math.min(1, (w * 0.56) / R));
      const at = (u: number, rho: number): [number, number, number] => {
        const th = -Math.PI / 2 + u * span;
        return [cx + rho * R * Math.cos(th), cy + rho * R * Math.sin(th), th];
      };
      const atm = ctx.createRadialGradient(cx, cy, R, cx, cy, R + h * 0.09);
      atm.addColorStop(0, withAlpha(col.accentHover, 0.38));
      atm.addColorStop(1, withAlpha(col.accentHover, 0));
      ctx.fillStyle = atm;
      ctx.beginPath();
      ctx.arc(cx, cy, R + h * 0.09, 0, TAU);
      ctx.fill();
      ctx.fillStyle = col.accentPressed;
      ctx.beginPath();
      ctx.arc(cx, cy, R + 3, 0, TAU);
      ctx.fill();
      ctx.fillStyle = col.accent;
      ctx.beginPath();
      ctx.arc(cx, cy, R, 0, TAU);
      ctx.fill();
      ctx.save();
      ctx.beginPath();
      ctx.arc(cx, cy, R, 0, TAU);
      ctx.clip();
      for (const b of bands) {
        ctx.strokeStyle = b.light ? withAlpha(col.accentHover, 0.35) : withAlpha(col.crater, 0.32);
        ctx.lineWidth = b.w * R * 0.12 + 1;
        ctx.beginPath();
        ctx.arc(cx, cy, b.rho * R, -Math.PI / 2 - span, -Math.PI / 2 + span);
        ctx.stroke();
      }
      for (const d of dust) {
        const [x, y] = at(d.u, d.rho);
        if (y > h) continue;
        ctx.fillStyle = d.light ? withAlpha(col.accentHover, d.a) : withAlpha(col.crater, d.a);
        ctx.fillRect(x, y, 1.4, 1.4);
      }
      for (const c of craters) {
        const [x, y, th] = at(c.u, c.rho);
        if (y > h + 20) continue;
        const r = c.r * h * 1.4;
        const f = clamp((1 - c.rho) / 0.2, 0.16, 1); // foreshortened toward the rim
        ctx.save();
        ctx.translate(x, y);
        ctx.rotate(th + Math.PI / 2);
        ctx.fillStyle = withAlpha(col.crater, 0.85);
        ctx.beginPath();
        ctx.ellipse(0, 0, r, r * f, 0, 0, TAU);
        ctx.fill();
        ctx.fillStyle = withAlpha(col.accentPressed, 0.9);
        ctx.beginPath();
        ctx.ellipse(r * 0.12, r * f * 0.18, r * 0.78, r * f * 0.7, 0, 0, TAU);
        ctx.fill();
        ctx.strokeStyle = withAlpha(col.accentHover, 0.8);
        ctx.lineWidth = Math.max(1, r * 0.12);
        ctx.beginPath();
        ctx.ellipse(0, 0, r, r * f, 0, Math.PI * 1.05, Math.PI * 1.85);
        ctx.stroke();
        ctx.restore();
      }
      const side = ctx.createLinearGradient(0, 0, w, 0);
      side.addColorStop(0, withAlpha(col.night, 0));
      side.addColorStop(0.55, withAlpha(col.night, 0.05));
      side.addColorStop(1, withAlpha(col.night, 0.38));
      ctx.fillStyle = side;
      ctx.fillRect(0, horizon - 4, w, h);
      const depth = ctx.createLinearGradient(0, horizon, 0, h);
      depth.addColorStop(0, withAlpha(col.night, 0));
      depth.addColorStop(1, withAlpha(col.night, 0.6));
      ctx.fillStyle = depth;
      ctx.fillRect(0, horizon - 4, w, h);
      ctx.restore();
      ctx.strokeStyle = withAlpha(col.accentHover, 0.9);
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(cx, cy, R, -Math.PI / 2 - span, -Math.PI / 2 + span * 0.2);
      ctx.stroke();

      // orbit: dashed path, solid where the sync has got to
      const orx = w * 0.43;
      const ory = horizon * 0.78;
      const a0 = Math.PI * 1.04;
      const a1 = Math.PI * 1.96;
      ctx.setLineDash([3, 6]);
      ctx.strokeStyle = withAlpha(col.text, 0.22);
      ctx.lineWidth = 1.2;
      ctx.beginPath();
      ctx.ellipse(cx, horizon, orx, ory, 0, a0, a1);
      ctx.stroke();
      ctx.setLineDash([]);
      const ap = a0 + (a1 - a0) * shown;
      ctx.strokeStyle = col.accentHover;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.ellipse(cx, horizon, orx, ory, 0, a0, ap);
      ctx.stroke();
      const sx = cx + orx * Math.cos(ap);
      const sy = horizon + ory * Math.sin(ap);

      // ground station: hunts while connecting, then tracks with a slight eased lag (no overshoot)
      const U = h * 0.085;
      const bx = cx;
      const by = horizon + U * 0.08;
      const px = bx;
      const py = by - U * 1.02;
      let target = Math.atan2(sy - py, sx - px) + Math.PI / 2;
      if (frame.phase === 'connect' && !reduced) target += Math.sin(t * 1.4) * 0.32;
      target = clamp(target, -1.3, 1.3);
      dishAng = reduced ? target : dishAng + (target - dishAng) * (1 - Math.exp(-dt * (frame.phase === 'connect' ? 2 : 3.2)));
      const r = U * 1.02;
      const d = r * 0.88; // the pivot sits at the back of the bowl
      const local = (lx: number, ly: number): [number, number] => [
        px + lx * Math.cos(dishAng) - ly * Math.sin(dishAng),
        py + lx * Math.sin(dishAng) + ly * Math.cos(dishAng),
      ];
      const [fx, fy] = local(0, -d - r * 0.86);

      // signal: wavefronts from the satellite, packets down to the feed
      const busy = frame.phase === 'channels' || frame.phase === 'guide';
      if (!done && frame.phase !== 'connect' && !reduced) {
        const toDish = Math.atan2(fy - sy, fx - sx);
        ctx.lineCap = 'round';
        for (let i = 0; i < 3; i++) {
          const k = (t * 0.55 + i / 3) % 1;
          ctx.strokeStyle = withAlpha(col.accentHover, (1 - k) * 0.45);
          ctx.lineWidth = 1.6;
          ctx.beginPath();
          ctx.arc(sx, sy, h * (0.04 + k * 0.2), toDish - 0.32, toDish + 0.32);
          ctx.stroke();
        }
        const n = busy ? 7 : 3;
        for (let i = 0; i < n; i++) {
          const k = (t * (busy ? 0.85 : 0.45) + i / n) % 1;
          if (k > 0.97) ledFor = 0.11;
          const c = i % 3 === 0 ? col.cyan : i % 3 === 1 ? col.blue : col.text;
          ctx.fillStyle = withAlpha(c, 0.95 * Math.sin(k * Math.PI));
          ctx.beginPath();
          ctx.arc(sx + (fx - sx) * k, sy + (fy - sy) * k, 1.6 + scale * 0.8, 0, TAU);
          ctx.fill();
        }
      }

      // pedestal, turntable, yoke
      ctx.fillStyle = col.mount;
      ctx.beginPath();
      ctx.moveTo(bx - U * 0.5, by);
      ctx.lineTo(bx - U * 0.2, by - U * 0.5);
      ctx.lineTo(bx + U * 0.2, by - U * 0.5);
      ctx.lineTo(bx + U * 0.5, by);
      ctx.closePath();
      ctx.fill();
      ctx.fillStyle = withAlpha(col.night, 0.35);
      ctx.beginPath();
      ctx.moveTo(bx + U * 0.05, by - U * 0.5);
      ctx.lineTo(bx + U * 0.2, by - U * 0.5);
      ctx.lineTo(bx + U * 0.5, by);
      ctx.lineTo(bx + U * 0.18, by);
      ctx.closePath();
      ctx.fill();
      ctx.fillStyle = col.accentPressed;
      ctx.beginPath();
      ctx.ellipse(bx, by - U * 0.52, U * 0.36, U * 0.09, 0, 0, TAU);
      ctx.fill();
      ctx.strokeStyle = withAlpha(col.accentHover, 0.7);
      ctx.lineWidth = 1;
      ctx.stroke();
      ctx.fillStyle = col.mount;
      rrect(ctx, bx - U * 0.09, py, U * 0.18, by - U * 0.52 - py, U * 0.04);
      ctx.fill();

      // elevation assembly: counterweight, bowl with its concave face, feed struts and horn
      ctx.save();
      ctx.translate(px, py);
      ctx.rotate(dishAng);
      ctx.fillStyle = col.mount;
      rrect(ctx, -U * 0.17, U * 0.1, U * 0.34, U * 0.22, U * 0.05);
      ctx.fill();
      ctx.fillRect(-U * 0.05, -d, U * 0.1, d + U * 0.12);
      const bowl = ctx.createLinearGradient(-r, 0, r, 0);
      bowl.addColorStop(0, col.accentHover);
      bowl.addColorStop(0.55, col.accent);
      bowl.addColorStop(1, col.accentPressed);
      ctx.fillStyle = bowl;
      ctx.beginPath();
      ctx.moveTo(-r, -d);
      ctx.arc(0, -d, r * 0.99, Math.PI, 0, true);
      ctx.closePath();
      ctx.fill();
      ctx.fillStyle = withAlpha(col.mount, 0.45);
      ctx.beginPath();
      ctx.moveTo(-r, -d);
      ctx.arc(0, -d, r * 0.99, Math.PI, 0, true);
      ctx.arc(0, -d + r * 0.08, r * 0.9, 0, Math.PI, false);
      ctx.closePath();
      ctx.fill();
      ctx.fillStyle = col.crater;
      ctx.beginPath();
      ctx.ellipse(0, -d, r, r * 0.14, 0, 0, TAU);
      ctx.fill();
      ctx.strokeStyle = col.accentHover;
      ctx.lineWidth = Math.max(1.5, U * 0.05);
      ctx.stroke();
      ctx.strokeStyle = col.text;
      ctx.lineWidth = Math.max(1.4, U * 0.055);
      ctx.lineJoin = 'round';
      ctx.lineCap = 'round';
      ctx.beginPath();
      ctx.moveTo(-r * 0.9, -d);
      ctx.lineTo(0, -d - r * 0.82);
      ctx.lineTo(r * 0.9, -d);
      ctx.moveTo(0, -d);
      ctx.lineTo(0, -d - r * 0.82);
      ctx.stroke();
      ctx.fillStyle = col.text;
      ctx.beginPath();
      ctx.moveTo(-U * 0.11, -d - r * 0.8);
      ctx.lineTo(U * 0.11, -d - r * 0.8);
      ctx.lineTo(U * 0.07, -d - r * 0.95);
      ctx.lineTo(-U * 0.07, -d - r * 0.95);
      ctx.closePath();
      ctx.fill();
      const led = ledFor > 0 || (frame.phase === 'connect' && Math.sin(t * 6) > 0.6);
      ctx.fillStyle = led ? col.cyan : withAlpha(col.cyan, 0.25);
      ctx.beginPath();
      ctx.arc(0, -d - r * 0.99, U * 0.055, 0, TAU);
      ctx.fill();
      if (led) {
        ctx.fillStyle = withAlpha(col.cyan, 0.25);
        ctx.beginPath();
        ctx.arc(0, -d - r * 0.99, U * 0.16, 0, TAU);
        ctx.fill();
      }
      ctx.restore();
      ctx.fillStyle = col.text;
      ctx.beginPath();
      ctx.arc(px, py, U * 0.12, 0, TAU);
      ctx.fill();
      ctx.fillStyle = col.mount;
      ctx.beginPath();
      ctx.arc(px, py, U * 0.05, 0, TAU);
      ctx.fill();

      // satellite: slow tumble and a blinking light
      satellite(ctx, sx, sy, h * 0.028, Math.sin(t * 0.6) * 0.2 + 0.35);
      if (Math.sin(t * 3) > 0.85) {
        ctx.fillStyle = col.cyan;
        ctx.beginPath();
        ctx.arc(sx + h * 0.07, sy - h * 0.015, 1.8, 0, TAU);
        ctx.fill();
      }
      // lock-on ring when the sync completes
      if (done && !reduced && doneFor < 0.6) {
        const k = doneFor / 0.6;
        ctx.strokeStyle = withAlpha(col.accentHover, 1 - k);
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(fx, fy, 6 + k * 50, 0, TAU);
        ctx.stroke();
      }
    },
  };
}

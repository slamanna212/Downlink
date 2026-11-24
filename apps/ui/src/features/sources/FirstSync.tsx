import { useEffect, useRef } from 'react';
import { t } from '../../lib/i18n';
import type { SyncView } from './syncProgress';
import { createUplinkScene, readPalette, type UplinkScene } from './uplinkScene';
import classes from './FirstSync.module.css';

/** Phase line and count for the status strip. */
export function syncLabel(v: SyncView, sourceName: string): { phase: string; count: string } {
  const n = (x: number) => x.toLocaleString('en');
  switch (v.phase) {
    case 'connect':
      return { phase: t('sync.connect', { name: sourceName }), count: '' };
    case 'channels':
      return {
        phase: t('sync.channels'),
        count: v.channels === 0 ? '' : v.channelsTotal > 0 ? t('sync.channelsOf', { done: n(v.channels), total: n(v.channelsTotal) }) : t('sync.channelsSoFar', { done: n(v.channels) }),
      };
    case 'guide':
      return {
        phase: t('sync.guide'),
        count: v.guideDone === 0 ? '' : v.guideTotal > 0 ? t('sync.percent', { pct: Math.round((v.guideDone / v.guideTotal) * 100) }) : t('sync.programmes', { done: n(v.guideDone) }),
      };
    case 'state':
      return { phase: t('sync.state'), count: '' };
    case 'done':
      return { phase: t('sync.done'), count: v.channels > 0 ? t('sync.channelsSoFar', { done: n(v.channels) }) : '' };
  }
}

/**
 * The first-sync screen (DESIGN §12): the Uplink scene full bleed, with the phase, count and progress
 * bar along the bottom. Shown by AddSourcePage while `add_source` runs its first sync.
 */
export function FirstSync({ view, sourceName }: { view: SyncView; sourceName: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const viewRef = useRef(view);
  useEffect(() => {
    viewRef.current = view;
  }, [view]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;
    const reduced = typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    const scene: UplinkScene = createUplinkScene(readPalette(canvas), reduced);
    let w = 0;
    let h = 0;
    const ro = new ResizeObserver(() => {
      const r = canvas.getBoundingClientRect();
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      w = r.width;
      h = r.height;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    });
    ro.observe(canvas);
    let raf = 0;
    let last = 0;
    let clock = 0;
    const frame = (now: number) => {
      const dt = last ? Math.min(0.1, (now - last) / 1000) : 0;
      last = now;
      clock += dt;
      if (w > 0 && h > 0) scene.draw(ctx, w, h, reduced ? 0 : clock, dt, viewRef.current);
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    // rAF already stops in a hidden window; reset the clock so the scene doesn't jump on return.
    const onVisible = () => {
      last = 0;
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, []);

  const { phase, count } = syncLabel(view, sourceName);
  const pct = Math.round(view.overall * 100);
  return (
    <div className={classes.root}>
      <canvas ref={canvasRef} className={classes.canvas} aria-hidden />
      <div className={classes.hud}>
        <h1 className={classes.title}>{t('sync.title')}</h1>
        <div className={classes.row} role="status" aria-live="polite">
          <span className={classes.phase}>{phase}</span>
          <span className={classes.count}>{count}</span>
        </div>
        <div className={classes.bar} role="progressbar" aria-label={t('sync.title')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
          <div className={classes.fill} style={{ transform: `scaleX(${view.overall})` }} />
        </div>
        <p className={classes.hint}>{t('sync.hint')}</p>
      </div>
    </div>
  );
}

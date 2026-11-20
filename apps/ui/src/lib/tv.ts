// 10-foot mode detection (PLAN §8.1). Explicit user density always wins; this only picks the default.
export interface TvSignals {
  /** Window width in CSS px. */
  width: number;
  /** `--tv` launch flag (the shell injects `window.__DL_TV__`) or `?tv` in the URL. */
  flag: boolean;
  /** No hover-capable fine pointer (remote, touch). */
  coarse: boolean;
}

export function wantsTv({ width, flag, coarse }: TvSignals): boolean {
  return flag || width >= 2560 || (width >= 1920 && coarse);
}

export function readTvSignals(): TvSignals {
  const w = window as unknown as { __DL_TV__?: boolean };
  let flag = w.__DL_TV__ === true;
  try {
    flag ||= new URLSearchParams(window.location.search).has('tv');
  } catch {
    /* no location */
  }
  const coarse = typeof window.matchMedia === 'function' && window.matchMedia('(hover: none), (pointer: coarse)').matches;
  return { width: window.innerWidth, flag, coarse };
}

/**
 * Live-edge rules (pure): when a live stream counts as "live", when the Go live button shows, where it
 * seeks, and how the timeline position is smoothed between player updates.
 */

/** Cushion kept behind the buffer tip: Go live lands here, not on the tip, so playback has data to lean on. */
export const LIVE_CUSHION_SECS = 3;
/** Behind live by more than this and the stream no longer counts as live (Go live appears). */
export const LIVE_SHOW_SECS = 10;
/** Once shown, Go live stays until we are back within this (hysteresis: stops flicker around the threshold). */
export const LIVE_HIDE_SECS = 6;
/** A gap between the smoothed and the reported position beyond this is a real seek: snap, don't glide. */
export const SMOOTH_SNAP_SECS = 4;

/** Whether "behind live" is shown, given the previous answer (hysteresis between LIVE_HIDE_SECS and LIVE_SHOW_SECS). */
export const isBehindLive = (behindSecs: number, wasBehind: boolean): boolean =>
  behindSecs > (wasBehind ? LIVE_HIDE_SECS : LIVE_SHOW_SECS);

/** Where Go live seeks: the live edge minus the cushion, never earlier than the oldest playable moment. */
export const goLiveTarget = (liveEdgeUtc: number, oldestUtc?: number | null): number => {
  const target = liveEdgeUtc - LIVE_CUSHION_SECS;
  return oldestUtc != null ? Math.max(oldestUtc, target) : target;
};

/**
 * One animation step of a position that should advance at real time and only drift gently towards the reported
 * value. While playing it never moves backwards: the rate stays within 0.5x..1.5x. Large gaps (seeks) snap.
 */
export function smoothStep(prev: number | null, target: number, dtSecs: number, playing: boolean): number {
  if (prev === null || !playing || Math.abs(target - prev) > SMOOTH_SNAP_SECS) return target;
  const dt = Math.max(0, dtSecs);
  const correction = Math.max(-0.5 * dt, Math.min(0.5 * dt, (target - (prev + dt)) * 2 * dt));
  return prev + dt + correction;
}

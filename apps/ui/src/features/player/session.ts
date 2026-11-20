// Who keeps the playback session alive (CLAUDE rule 6). Each video surface (the full player, the guide's
// mini player) calls `holdSession()` on mount and `releaseSession()` on unmount. A release stops playback
// one tick later unless another surface holds it by then, so player -> guide (mini) and guide -> player
// hand the session over, StrictMode's simulated remount does not kill playback, and any other
// navigate-away still stops it.
import { invoke } from '../../api/ipc';
import type { PlayerPhase } from '../../api/types';

/** Phases where there is a picture (or one on the way) worth showing in a mini player. */
export const ACTIVE_PHASES: ReadonlySet<PlayerPhase> = new Set(['Resolving', 'Connecting', 'Buffering', 'Playing', 'Paused']);

let pendingStop: ReturnType<typeof setTimeout> | null = null;

export function holdSession(): void {
  if (pendingStop) clearTimeout(pendingStop);
  pendingStop = null;
}

export function releaseSession(): void {
  if (pendingStop) clearTimeout(pendingStop);
  pendingStop = setTimeout(() => {
    pendingStop = null;
    void invoke('player_command', { command: { kind: 'stop' } }).catch(() => undefined);
  }, 0);
}

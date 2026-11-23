// Who keeps the playback session alive (CLAUDE rule 6). Each video surface (the full player, the guide's
// mini player, multiview) calls `holdSession()` on mount and `releaseSession()` on unmount. Holds are
// counted; a release checks one tick later and stops playback only if no surface holds it by then. So
// player -> guide (mini) and guide -> player hand the session over, StrictMode's simulated remount does
// not kill playback, and any other navigate-away still stops it.
//
// Counting (rather than "a hold cancels a pending stop") matters because React runs the incoming page's
// layout effects BEFORE the outgoing page's passive-effect cleanup: player -> guide calls the mini
// player's hold first and the player's release second, which must not stop playback.
import { invoke } from '../../api/ipc';
import type { PlayerPhase } from '../../api/types';

/** Phases where there is a picture (or one on the way) worth showing in a mini player. */
export const ACTIVE_PHASES: ReadonlySet<PlayerPhase> = new Set(['Resolving', 'Connecting', 'Buffering', 'Playing', 'Paused']);

let holders = 0;
let pendingStop: ReturnType<typeof setTimeout> | null = null;

export function holdSession(): void {
  holders += 1;
}

export function releaseSession(): void {
  holders = Math.max(0, holders - 1);
  if (pendingStop) clearTimeout(pendingStop);
  pendingStop = setTimeout(() => {
    pendingStop = null;
    if (holders > 0) return;
    void invoke('player_command', { command: { kind: 'stop' } }).catch(() => undefined);
  }, 0);
}

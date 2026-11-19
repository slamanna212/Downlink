import { create } from 'zustand';
import { invoke, listen } from '../api/ipc';
import type { PlayerState } from '../api/types';

const IDLE: PlayerState = {
  state: 'Idle', mode: null, channelId: null, position: null, duration: null, buffered: null, seekable: false,
  volume: 100, muted: false, hwdecCurrent: null, videoCodec: null, width: null, height: null, waitingOn: null,
  error: null, transitionedAt: 0, programme: null, resumeAt: null,
};

interface PlayerStore {
  player: PlayerState;
  setPlayer: (p: PlayerState) => void;
}

export const usePlayerStore = create<PlayerStore>((set) => ({
  player: IDLE,
  setPlayer: (player) => set({ player }),
}));

/** Subscribe to `player://state` (<=10 Hz from Rust), coalescing to one store write per frame. */
export async function startPlayerBridge(): Promise<() => void> {
  let pending: PlayerState | null = null;
  let raf = 0;
  const flush = () => {
    raf = 0;
    if (pending) {
      usePlayerStore.getState().setPlayer(pending);
      pending = null;
    }
  };
  const schedule = () => {
    if (raf) return;
    raf = typeof requestAnimationFrame === 'function' ? requestAnimationFrame(flush) : (setTimeout(flush, 16) as unknown as number);
  };
  const unlisten = await listen('player://state', (p) => {
    pending = p;
    schedule();
  });
  try {
    usePlayerStore.getState().setPlayer(await invoke('get_player_state', {}));
  } catch {
    /* initial state is best-effort; events will follow */
  }
  return () => {
    unlisten();
    if (raf && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(raf);
  };
}

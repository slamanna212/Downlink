import { beforeEach, describe, expect, it } from 'vitest';
import { usePlayerStore } from './player';
import type { PlayerState } from '../api/types';

const base = usePlayerStore.getState().player;
const state = (over: Partial<PlayerState>): PlayerState => ({ ...base, ...over });

describe('player store', () => {
  beforeEach(() => usePlayerStore.getState().setPlayer(base));

  it('leaves Failed as soon as the next state arrives and drops the old error', () => {
    const { setPlayer } = usePlayerStore.getState();
    setPlayer(state({ state: 'Failed', channelId: 1, error: { code: 'stream', message: 'boom' } as PlayerState['error'] }));
    expect(usePlayerStore.getState().player.state).toBe('Failed');
    setPlayer(state({ state: 'Connecting', channelId: 2, error: null }));
    const p = usePlayerStore.getState().player;
    expect(p.state).toBe('Connecting');
    expect(p.channelId).toBe(2);
    expect(p.error).toBeNull();
  });
});

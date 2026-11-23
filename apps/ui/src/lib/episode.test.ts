import { describe, expect, it } from 'vitest';
import { episodeCode, episodeLine } from './episode';

describe('episode labels', () => {
  it('codes', () => {
    expect(episodeCode({ season: 2, episode: 5 })).toBe('S2 E5');
    expect(episodeCode({ season: null, episode: 5 })).toBe('E5');
    expect(episodeCode({ season: 2, episode: null })).toBeNull();
    expect(episodeCode({ season: 0, episode: 0 })).toBeNull();
  });
  it('lines', () => {
    expect(episodeLine({ season: 2, episode: 5, subtitle: 'Low Tide' })).toBe('S2 E5 · Low Tide');
    expect(episodeLine({ season: null, episode: null, subtitle: 'Low Tide' })).toBe('Low Tide');
    expect(episodeLine({ season: 1, episode: 3, subtitle: 'Episode 3' })).toBe('S1 E3');
    expect(episodeLine({ season: 1, episode: 3, subtitle: 'S01E03' })).toBe('S1 E3');
    expect(episodeLine({ episodeTitle: 'Paper Boats', season: 1, episode: 2 })).toBe('S1 E2 · Paper Boats');
    expect(episodeLine({ season: null, episode: null, subtitle: '  ' })).toBeNull();
  });
});

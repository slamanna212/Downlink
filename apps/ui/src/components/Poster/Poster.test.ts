import { describe, expect, it } from 'vitest';
import { titleHash } from './Poster';

describe('titleHash', () => {
  it('is deterministic and spreads', () => {
    expect(titleHash('Silent Harbour')).toBe(titleHash('Silent Harbour'));
    expect(new Set(['a', 'b', 'c', 'd', 'e', 'f'].map((s) => titleHash(s) % 4)).size).toBeGreaterThan(1);
  });
});

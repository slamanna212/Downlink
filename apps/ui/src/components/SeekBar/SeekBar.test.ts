import { describe, expect, it } from 'vitest';
import { positionFromRatio } from './SeekBar';

describe('positionFromRatio', () => {
  it('maps and clamps', () => {
    expect(positionFromRatio(0.5, 3600)).toBe(1800);
    expect(positionFromRatio(-1, 3600)).toBe(0);
    expect(positionFromRatio(2, 3600)).toBe(3600);
    expect(positionFromRatio(0.5, 0)).toBe(0);
  });
});

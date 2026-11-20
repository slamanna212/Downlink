import { describe, expect, it } from 'vitest';
import { wantsTv } from './tv';

describe('wantsTv', () => {
  it('honours the flag, 4K-wide windows and big coarse-pointer windows', () => {
    expect(wantsTv({ width: 1280, flag: true, coarse: false })).toBe(true);
    expect(wantsTv({ width: 2560, flag: false, coarse: false })).toBe(true);
    expect(wantsTv({ width: 1920, flag: false, coarse: true })).toBe(true);
    expect(wantsTv({ width: 1920, flag: false, coarse: false })).toBe(false);
    expect(wantsTv({ width: 1440, flag: false, coarse: true })).toBe(false);
  });
});

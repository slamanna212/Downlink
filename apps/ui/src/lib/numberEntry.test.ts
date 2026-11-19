import { describe, expect, it } from 'vitest';
import { appendDigit, matchChannelNumber } from './numberEntry';

const ch = [
  { id: 1, number: 1 },
  { id: 2, number: 5.5 },
  { id: 3, number: 12 },
  { id: 4, number: 7 },
  { id: 5, number: null },
  { id: 6, number: 100 },
];

describe('matchChannelNumber', () => {
  it('prefers an exact match', () => {
    expect(matchChannelNumber('12', ch)?.id).toBe(3);
    expect(matchChannelNumber('1', ch)?.id).toBe(1);
  });
  it('falls back to the lowest number >= entry', () => {
    expect(matchChannelNumber('6', ch)?.id).toBe(4);
    expect(matchChannelNumber('5', ch)?.id).toBe(2);
    expect(matchChannelNumber('13', ch)?.id).toBe(6);
  });
  it('returns null when nothing is >= entry or entry is invalid', () => {
    expect(matchChannelNumber('101', ch)).toBeNull();
    expect(matchChannelNumber('', ch)).toBeNull();
    expect(matchChannelNumber('1x', ch)).toBeNull();
  });
  it('does not depend on list order', () => {
    expect(matchChannelNumber('6', [...ch].reverse())?.id).toBe(4);
  });
});

describe('appendDigit', () => {
  it('accepts digits only and caps length', () => {
    expect(appendDigit('1', '2')).toBe('12');
    expect(appendDigit('1', 'a')).toBe('1');
    expect(appendDigit('12345', '6')).toBe('12345');
  });
});

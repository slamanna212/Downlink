import { describe, expect, it } from 'vitest';
import { isTypeaheadKey, nextIndex } from './listNav';

const k = (key: string, mods: Partial<{ ctrlKey: boolean; metaKey: boolean; altKey: boolean }> = {}) => ({
  key, ctrlKey: false, metaKey: false, altKey: false, ...mods,
});

describe('nextIndex', () => {
  it('moves and clamps', () => {
    expect(nextIndex(0, 'ArrowDown', 5)).toBe(1);
    expect(nextIndex(4, 'ArrowDown', 5)).toBe(4);
    expect(nextIndex(0, 'ArrowUp', 5)).toBe(0);
    expect(nextIndex(3, 'ArrowUp', 5)).toBe(2);
  });
  it('starts at the first row from no selection', () => {
    expect(nextIndex(-1, 'ArrowDown', 5)).toBe(0);
  });
  it('pages, homes and ends', () => {
    expect(nextIndex(2, 'PageDown', 100, 8)).toBe(10);
    expect(nextIndex(95, 'PageDown', 100, 8)).toBe(99);
    expect(nextIndex(5, 'PageUp', 100, 8)).toBe(0);
    expect(nextIndex(40, 'Home', 100)).toBe(0);
    expect(nextIndex(40, 'End', 100)).toBe(99);
  });
  it('handles empty lists', () => {
    expect(nextIndex(0, 'ArrowDown', 0)).toBe(-1);
  });
});

describe('isTypeaheadKey', () => {
  it('accepts plain printable characters only', () => {
    expect(isTypeaheadKey(k('a'))).toBe(true);
    expect(isTypeaheadKey(k('7'))).toBe(true);
    expect(isTypeaheadKey(k('/'))).toBe(false);
    expect(isTypeaheadKey(k(' '))).toBe(false);
    expect(isTypeaheadKey(k('Enter'))).toBe(false);
    expect(isTypeaheadKey(k('a', { ctrlKey: true }))).toBe(false);
    expect(isTypeaheadKey(k('k', { metaKey: true }))).toBe(false);
  });
});

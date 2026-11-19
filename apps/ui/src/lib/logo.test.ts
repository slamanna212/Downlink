import { describe, expect, it } from 'vitest';
import { logoPalette } from '../theme';
import { hashChannelId, initialsOf, logoColor, logoPaletteIndex } from './logo';

describe('logo fallback', () => {
  it('hashes deterministically (pinned values)', () => {
    // FNV-1a 32-bit reference values; if these change, every channel's colour changes.
    expect(hashChannelId('')).toBe(0x811c9dc5);
    expect(hashChannelId('a')).toBe(0xe40c292c);
    expect(hashChannelId('foobar')).toBe(0xbf9cf968);
  });

  it('treats numeric and string ids identically', () => {
    expect(hashChannelId(123)).toBe(hashChannelId('123'));
  });

  it('maps ids into the 8-colour palette', () => {
    for (let i = 0; i < 500; i++) {
      const idx = logoPaletteIndex(i);
      expect(idx).toBeGreaterThanOrEqual(0);
      expect(idx).toBeLessThan(8);
      expect(logoColor(i)).toBe(logoPalette[idx]);
    }
  });

  it('spreads ids across the palette', () => {
    const seen = new Set(Array.from({ length: 200 }, (_, i) => logoPaletteIndex(i)));
    expect(seen.size).toBe(8);
  });

  it('is stable for the same channel', () => {
    expect(logoColor('chan-42')).toBe(logoColor('chan-42'));
  });

  it('builds initials from the first two words', () => {
    expect(initialsOf('Harbor News 24')).toBe('HN');
    expect(initialsOf('  cobalt   sports ')).toBe('CS');
    expect(initialsOf('Atlas')).toBe('A');
    expect(initialsOf('')).toBe('?');
    expect(initialsOf('Édith Piaf TV')).toBe('ÉP');
  });
});

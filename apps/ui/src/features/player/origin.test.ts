import { describe, expect, it } from 'vitest';
import { originLabel } from './origin';

describe('originLabel', () => {
  it('names the route the player was opened from', () => {
    expect(originLabel('/guide', true)).toBe('Guide');
    expect(originLabel('/', true)).toBe('Home');
    expect(originLabel('/channels', true)).toBe('Channels');
    expect(originLabel('/movies/12', true)).toBe('Movies');
  });
  it('falls back to Back for unknown routes and to Channels without history', () => {
    expect(originLabel('/somewhere', true)).toBe('Back');
    expect(originLabel(null, true)).toBe('Back');
    expect(originLabel('/guide', false)).toBe('Channels');
  });
});

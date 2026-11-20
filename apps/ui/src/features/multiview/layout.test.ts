import { describe, expect, it } from 'vitest';
import { availableLayouts, fitLayout, gridArea, layoutSize, nextLayout } from './layout';

describe('multiview layout', () => {
  it('sizes layouts', () => {
    expect([layoutSize('2-up'), layoutSize('2x2'), layoutSize('1+3'), layoutSize('3x3')]).toEqual([2, 4, 4, 9]);
  });

  it('offers only layouts within the cap that fit the occupied slots', () => {
    expect(availableLayouts(4, 1)).toEqual(['2-up', '2x2', '1+3']);
    expect(availableLayouts(9, 3)).toEqual(['2x2', '1+3', '3x3']);
    expect(availableLayouts(2, 1)).toEqual(['2-up']);
    expect(availableLayouts(1, 1)).toEqual([]);
  });

  it('fits the smallest layout', () => {
    expect(fitLayout(2, 4)).toBe('2-up');
    expect(fitLayout(3, 4)).toBe('2x2');
    expect(fitLayout(6, 9)).toBe('3x3');
    expect(fitLayout(6, 4)).toBe('2x2');
  });

  it('places tiles', () => {
    expect(gridArea('1+3', 0)).toEqual({ gridColumn: '1 / 2', gridRow: '1 / 4' });
    expect(gridArea('1+3', 2)).toEqual({ gridColumn: '2 / 3', gridRow: '2 / 3' });
    expect(gridArea('3x3', 4)).toEqual({ gridColumn: '2 / 3', gridRow: '2 / 3' });
    expect(gridArea('2x2', 3)).toEqual({ gridColumn: '2 / 3', gridRow: '2 / 3' });
  });

  it('cycles layouts', () => {
    expect(nextLayout('2x2', ['2-up', '2x2', '1+3'])).toBe('1+3');
    expect(nextLayout('1+3', ['2-up', '2x2', '1+3'])).toBe('2-up');
  });
});

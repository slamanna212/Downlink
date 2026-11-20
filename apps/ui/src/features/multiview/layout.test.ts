import { describe, expect, it } from 'vitest';
import { gridRows, layoutFor, nextStyle, stylesFor } from './layout';

describe('multiview layout', () => {
  it('offers a featured option only where one exists', () => {
    expect(stylesFor(1)).toEqual(['grid']);
    expect(stylesFor(2)).toEqual(['grid']);
    expect(stylesFor(3)).toEqual(['grid', 'featured']);
    expect(stylesFor(4)).toEqual(['grid', 'featured']);
    expect(stylesFor(7)).toEqual(['grid']);
    expect(stylesFor(9)).toEqual(['grid']);
  });

  it('balances grid rows, longer rows first', () => {
    expect([1, 2, 3, 4, 5, 6, 7, 8, 9].map(gridRows)).toEqual([[1], [2], [2, 1], [2, 2], [3, 2], [3, 3], [3, 2, 2], [3, 3, 2], [3, 3, 3]]);
  });

  it('fills the view with one tile and puts two side by side', () => {
    expect(layoutFor(1, 'grid').areas).toEqual([{ gridColumn: '1 / 3', gridRow: '1 / 2' }]);
    const two = layoutFor(2, 'grid');
    expect(two.gridTemplateRows).toBe('repeat(1, 1fr)');
    expect(two.areas).toEqual([
      { gridColumn: '1 / 3', gridRow: '1 / 2' },
      { gridColumn: '3 / 5', gridRow: '1 / 2' },
    ]);
  });

  it('centres a short last row', () => {
    expect(layoutFor(3, 'grid').areas[2]).toEqual({ gridColumn: '2 / 4', gridRow: '2 / 3' });
    expect(layoutFor(5, 'grid').areas.slice(3)).toEqual([
      { gridColumn: '2 / 4', gridRow: '2 / 3' },
      { gridColumn: '4 / 6', gridRow: '2 / 3' },
    ]);
  });

  it('places featured layouts', () => {
    expect(layoutFor(4, 'featured').areas[0]).toEqual({ gridColumn: '1 / 2', gridRow: '1 / 4' });
    expect(layoutFor(4, 'featured').areas[2]).toEqual({ gridColumn: '2 / 3', gridRow: '2 / 3' });
    expect(layoutFor(5, 'featured').areas[0]).toEqual({ gridColumn: '1 / 5', gridRow: '1 / 2' });
    const six = layoutFor(6, 'featured');
    expect(six.areas).toHaveLength(6);
    expect(six.areas[0]).toEqual({ gridColumn: '1 / 3', gridRow: '1 / 3' });
    expect(six.areas[5]).toEqual({ gridColumn: '3 / 4', gridRow: '3 / 4' });
    expect(layoutFor(8, 'featured').areas).toHaveLength(8);
  });

  it('every layout has one distinct area per tile', () => {
    for (let n = 1; n <= 9; n++) {
      for (const s of stylesFor(n)) {
        const areas = layoutFor(n, s).areas.map((a) => `${a.gridColumn}|${a.gridRow}`);
        expect(new Set(areas).size).toBe(n);
      }
    }
  });

  it('falls back to grid where featured does not exist', () => {
    expect(layoutFor(2, 'featured')).toEqual(layoutFor(2, 'grid'));
  });

  it('cycles styles', () => {
    expect(nextStyle('grid', ['grid', 'featured'])).toBe('featured');
    expect(nextStyle('featured', ['grid', 'featured'])).toBe('grid');
    expect(nextStyle('featured', ['grid'])).toBe('grid');
  });
});

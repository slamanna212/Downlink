// Multiview layouts (DESIGN.md §9). The grid follows the number of tiles on screen: one tile fills the
// view, two sit side by side, more reflow into a balanced grid. Counts that also have a "featured"
// arrangement (one large tile plus smaller ones) offer it as an option. Pure helpers so the maths is testable.

export type LayoutStyle = 'grid' | 'featured';

export interface GridArea {
  gridColumn: string;
  gridRow: string;
}

export interface LayoutSpec {
  gridTemplateColumns: string;
  gridTemplateRows: string;
  /** Placement per position (0 = first tile in slot order; the main player when it is on screen). */
  areas: GridArea[];
}

const area = (c0: number, c1: number, r0: number, r1: number): GridArea => ({
  gridColumn: `${c0} / ${c1}`,
  gridRow: `${r0} / ${r1}`,
});

/** Layout styles that exist for `count` tiles; 'grid' always comes first. */
export function stylesFor(count: number): LayoutStyle[] {
  return [3, 4, 5, 6, 8].includes(count) ? ['grid', 'featured'] : ['grid'];
}

/** Tiles per row for an even grid: as square as possible, longer rows first. */
export function gridRows(count: number): number[] {
  if (count <= 1) return [1];
  const cols = Math.ceil(Math.sqrt(count));
  const rows = Math.ceil(count / cols);
  const base = Math.floor(count / rows);
  const extra = count % rows;
  return Array.from({ length: rows }, (_, i) => base + (i < extra ? 1 : 0));
}

/** Even grid; a shorter row is centred (each tile spans two half-columns). */
function grid(count: number): LayoutSpec {
  const rows = gridRows(count);
  const cols = Math.max(...rows);
  const areas: GridArea[] = [];
  rows.forEach((n, r) => {
    const start = cols - n + 1;
    for (let j = 0; j < n; j++) areas.push(area(start + 2 * j, start + 2 * j + 2, r + 1, r + 2));
  });
  return {
    gridTemplateColumns: `repeat(${cols * 2}, 1fr)`,
    gridTemplateRows: `repeat(${rows.length}, 1fr)`,
    areas,
  };
}

/** One large tile plus the rest: a side column (2–3 others), a bottom strip (4) or an L border (5, 7). */
function featured(count: number): LayoutSpec {
  const k = count - 1;
  if (k === 2 || k === 3) {
    return {
      gridTemplateColumns: k === 2 ? '2fr 1fr' : '3fr 1fr',
      gridTemplateRows: `repeat(${k}, 1fr)`,
      areas: [area(1, 2, 1, k + 1), ...Array.from({ length: k }, (_, i) => area(2, 3, i + 1, i + 2))],
    };
  }
  if (k === 4) {
    return {
      gridTemplateColumns: 'repeat(4, 1fr)',
      gridTemplateRows: '3fr 1fr',
      areas: [area(1, 5, 1, 2), ...Array.from({ length: 4 }, (_, i) => area(i + 1, i + 2, 2, 3))],
    };
  }
  // L border: the large tile is s×s in an (s+1)×(s+1) grid; right column top-down, then the bottom row.
  const s = (k - 1) / 2;
  const areas = [area(1, s + 1, 1, s + 1)];
  for (let r = 1; r <= s; r++) areas.push(area(s + 1, s + 2, r, r + 1));
  for (let c = 1; c <= s + 1; c++) areas.push(area(c, c + 1, s + 1, s + 2));
  return {
    gridTemplateColumns: `repeat(${s + 1}, 1fr)`,
    gridTemplateRows: `repeat(${s + 1}, 1fr)`,
    areas,
  };
}

/** Grid template and per-position placement for `count` tiles; falls back to 'grid' when `style` does not exist. */
export function layoutFor(count: number, style: LayoutStyle): LayoutSpec {
  const n = Math.max(1, count);
  return style === 'featured' && stylesFor(n).includes('featured') ? featured(n) : grid(n);
}

/** Next style for `L`, cycling through the ones available. */
export function nextStyle(current: LayoutStyle, allowed: readonly LayoutStyle[]): LayoutStyle {
  if (allowed.length === 0) return current;
  const i = allowed.indexOf(current);
  return allowed[(i + 1) % allowed.length] ?? current;
}

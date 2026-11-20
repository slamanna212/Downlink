// Multiview layouts (DESIGN.md §9). Pure helpers so the grid maths is testable.

export type MultiviewLayout = '2-up' | '2x2' | '1+3' | '3x3';

export const LAYOUTS: readonly MultiviewLayout[] = ['2-up', '2x2', '1+3', '3x3'];

/** Tiles a layout shows, main player included. */
export function layoutSize(l: MultiviewLayout): number {
  return l === '2-up' ? 2 : l === '3x3' ? 9 : 4;
}

/** Layouts allowed with `maxTiles` tiles that still fit every occupied slot (highest slot index + 1). */
export function availableLayouts(maxTiles: number, occupied: number): MultiviewLayout[] {
  return LAYOUTS.filter((l) => layoutSize(l) <= maxTiles && layoutSize(l) >= occupied);
}

/** Smallest layout that fits `needed` tiles within `maxTiles`, else the largest allowed. */
export function fitLayout(needed: number, maxTiles: number): MultiviewLayout {
  const ok = LAYOUTS.filter((l) => layoutSize(l) <= maxTiles);
  const biggest = Math.max(0, ...ok.map(layoutSize));
  return ok.find((l) => layoutSize(l) >= needed) ?? ok.find((l) => layoutSize(l) === biggest) ?? '2-up';
}

/** CSS grid placement for `slot` in layout `l` (1-based lines). */
export function gridArea(l: MultiviewLayout, slot: number): { gridColumn: string; gridRow: string } {
  if (l === '1+3') {
    return slot === 0 ? { gridColumn: '1 / 2', gridRow: '1 / 4' } : { gridColumn: '2 / 3', gridRow: `${slot} / ${slot + 1}` };
  }
  const cols = l === '2-up' ? 2 : l === '3x3' ? 3 : 2;
  const col = (slot % cols) + 1;
  const row = Math.floor(slot / cols) + 1;
  return { gridColumn: `${col} / ${col + 1}`, gridRow: `${row} / ${row + 1}` };
}

/** Grid template for a layout. */
export function gridTemplate(l: MultiviewLayout): { gridTemplateColumns: string; gridTemplateRows: string } {
  switch (l) {
    case '2-up':
      return { gridTemplateColumns: '1fr 1fr', gridTemplateRows: '1fr' };
    case '2x2':
      return { gridTemplateColumns: '1fr 1fr', gridTemplateRows: '1fr 1fr' };
    case '1+3':
      return { gridTemplateColumns: '3fr 1fr', gridTemplateRows: '1fr 1fr 1fr' };
    case '3x3':
      return { gridTemplateColumns: '1fr 1fr 1fr', gridTemplateRows: '1fr 1fr 1fr' };
  }
}

/** Next layout for `L`, cycling through the allowed ones. */
export function nextLayout(current: MultiviewLayout, allowed: readonly MultiviewLayout[]): MultiviewLayout {
  if (allowed.length === 0) return current;
  const i = allowed.indexOf(current);
  return allowed[(i + 1) % allowed.length] ?? current;
}

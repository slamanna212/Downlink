import { logoPalette } from '../theme';

/** FNV-1a 32-bit over the UTF-16 code units of the id. Deterministic across runs and platforms. */
export function hashChannelId(id: string | number): number {
  const s = String(id);
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

export const logoPaletteIndex = (id: string | number): number => hashChannelId(id) % logoPalette.length;

/** Palette colour for a channel's fallback plate. */
export const logoColor = (id: string | number): string => logoPalette[logoPaletteIndex(id)] ?? logoPalette[0];

/** First letter of the first two words, uppercased. "Harbor News 24" -> "HN". */
export function initialsOf(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words
    .slice(0, 2)
    .map((w) => Array.from(w)[0] ?? '')
    .join('');
  return (letters || '?').toUpperCase();
}

// Channel number entry (PLAN §8.4: 0-9, 2 s timeout): exact match, else the lowest number >= entry.
export const NUMBER_ENTRY_TIMEOUT_MS = 2000;
export const MAX_ENTRY_DIGITS = 5;

export interface Numbered {
  id: number;
  number: number | null;
}

/** Append a digit; ignores anything else and caps the length. */
export function appendDigit(entry: string, key: string): string {
  if (!/^[0-9]$/.test(key) || entry.length >= MAX_ENTRY_DIGITS) return entry;
  return entry + key;
}

export function matchChannelNumber<T extends Numbered>(entry: string, channels: readonly T[]): T | null {
  if (!/^[0-9]+$/.test(entry)) return null;
  const target = Number(entry);
  let best: T | null = null;
  for (const c of channels) {
    if (c.number == null) continue;
    if (c.number === target) return c;
    if (c.number > target && (best === null || (best.number ?? Infinity) > c.number)) best = c;
  }
  return best;
}

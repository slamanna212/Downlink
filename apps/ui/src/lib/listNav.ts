// Pure keyboard logic for virtualised lists (kept out of components so it is unit-testable).
export type ListKey = 'ArrowUp' | 'ArrowDown' | 'PageUp' | 'PageDown' | 'Home' | 'End';

export const LIST_KEYS: readonly string[] = ['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End'];
export const isListKey = (k: string): k is ListKey => LIST_KEYS.includes(k);

/** Next active index for a navigation key; clamped, -1 when the list is empty. */
export function nextIndex(current: number, key: ListKey, length: number, page = 8): number {
  if (length <= 0) return -1;
  const cur = Math.min(Math.max(current, 0), length - 1);
  const clamp = (n: number) => Math.min(Math.max(n, 0), length - 1);
  switch (key) {
    case 'ArrowUp': return clamp(cur - 1);
    case 'ArrowDown': return current < 0 ? 0 : clamp(cur + 1);
    case 'PageUp': return clamp(cur - page);
    case 'PageDown': return clamp(cur + page);
    case 'Home': return 0;
    case 'End': return length - 1;
  }
}

/** A key that should start type-ahead filtering when nothing editable has focus. */
export function isTypeaheadKey(e: { key: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean }): boolean {
  return e.key.length === 1 && e.key !== ' ' && e.key !== '/' && !e.ctrlKey && !e.metaKey && !e.altKey;
}

export function isEditableTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  return t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName);
}

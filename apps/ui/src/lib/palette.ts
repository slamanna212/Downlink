// The command palette is code-split and mounted on idle; opening it before it has loaded is queued.
const EVENT = 'dl:open-palette';
let pending = false;

export function openPalette(): void {
  pending = true;
  window.dispatchEvent(new Event(EVENT));
}

/** Called by the palette: runs `open` for a queued request and for every later one. Returns the cleanup. */
export function onOpenPalette(open: () => void): () => void {
  const handler = () => {
    pending = false;
    open();
  };
  window.addEventListener(EVENT, handler);
  if (pending) handler();
  return () => window.removeEventListener(EVENT, handler);
}

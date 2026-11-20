// Tauri serves custom protocols as `http://<scheme>.localhost/...` on Windows, `<scheme>://...` elsewhere.
export const isWindows = (): boolean => typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent);

/** Rewrites `dlimg://hash/size` to the form the current platform's webview can load. Other URLs pass through. */
export function resolveImg(src: string | null | undefined, windows: boolean = isWindows()): string | null {
  if (!src) return null;
  return windows && src.startsWith('dlimg://') ? `http://dlimg.localhost/${src.slice('dlimg://'.length)}` : src;
}

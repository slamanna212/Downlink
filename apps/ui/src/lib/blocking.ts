import { create } from 'zustand';

// Database and certificate errors stop the app until the user decides (PLAN §7.6, §2.6).
export const NEWER_DB_MESSAGE = 'This library was created by a newer version of Downlink.';
export const CERT_CHANGED_MESSAGE = 'The server’s certificate changed.';
const CERT_UNTRUSTED = "This source has no trusted certificate.";
const CERT_ASCII = "The server's certificate changed.";

export type BlockingKind = 'newer-db' | 'cert' | 'missing-cert';

export function blockingKind(message: string): BlockingKind | null {
  if (message.startsWith(NEWER_DB_MESSAGE)) return 'newer-db';
  if (message.startsWith(CERT_UNTRUSTED)) return 'missing-cert';
  if (message.startsWith(CERT_CHANGED_MESSAGE) || message.startsWith(CERT_ASCII)) return 'cert';
  return null;
}

interface BlockingState {
  kind: BlockingKind | null;
  sourceId: number | null;
  show: (k: BlockingKind, sourceId?: number) => void;
  clear: () => void;
}

export const useBlockingStore = create<BlockingState>((set) => ({
  kind: null,
  sourceId: null,
  show: (kind, sourceId) => set({ kind, sourceId: sourceId ?? null }),
  clear: () => set({ kind: null, sourceId: null }),
}));

/** Raise the blocking screen for a database or certificate error. */
export function noteBlockingError(message: string, sourceId?: number): void {
  const kind = blockingKind(message);
  if (kind) useBlockingStore.getState().show(kind, sourceId);
}

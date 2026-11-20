import { create } from 'zustand';

// Two AppError messages are not toasts: they stop the app until the user decides (PLAN §7.6, §2.6).
export const NEWER_DB_MESSAGE = 'This library was created by a newer version of Downlink.';
export const CERT_CHANGED_MESSAGE = 'The server’s certificate changed.';
const CERT_ASCII = "The server's certificate changed.";

export type BlockingKind = 'newer-db' | 'cert';

export function blockingKind(message: string): BlockingKind | null {
  if (message.startsWith(NEWER_DB_MESSAGE)) return 'newer-db';
  if (message.startsWith(CERT_CHANGED_MESSAGE) || message.startsWith(CERT_ASCII)) return 'cert';
  return null;
}

interface BlockingState {
  kind: BlockingKind | null;
  show: (k: BlockingKind) => void;
  clear: () => void;
}

export const useBlockingStore = create<BlockingState>((set) => ({
  kind: null,
  show: (kind) => set({ kind }),
  clear: () => set({ kind: null }),
}));

/** Called for every failed command; raises the blocking screen when the message is one of the two. */
export function noteBlockingError(message: string): void {
  const kind = blockingKind(message);
  if (kind) useBlockingStore.getState().show(kind);
}

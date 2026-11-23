import { create } from 'zustand';
import type { Channel, Programme, Recording } from '../api/types';

/** What the details modal is about. The modal fills in the rest with `media_details`. */
export type InfoSubject =
  | { kind: 'programme'; programme: Programme; channel?: Channel }
  | { kind: 'recording'; recording: Recording };

interface InfoState {
  subject: InfoSubject | null;
  open: (s: InfoSubject) => void;
  close: () => void;
}

export const useInfoStore = create<InfoState>((set) => ({
  subject: null,
  open: (subject) => set({ subject }),
  close: () => set({ subject: null }),
}));

/** Open the details modal from anywhere (event handlers, non-React code). */
export const openInfo = (s: InfoSubject): void => useInfoStore.getState().open(s);

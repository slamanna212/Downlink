import { create } from 'zustand';

interface DetailState {
  /** The channel whose detail drawer is open (and an optional programme to highlight). */
  open: { channelId: number; programmeId?: string } | null;
  show: (channelId: number, programmeId?: string) => void;
  close: () => void;
}

export const useDetailStore = create<DetailState>((set) => ({
  open: null,
  show: (channelId, programmeId) => set({ open: { channelId, programmeId } }),
  close: () => set({ open: null }),
}));

import type { ReactNode } from 'react';
import { create } from 'zustand';
import type { Channel } from '../api/types';

interface ChannelMenuState {
  open: { x: number; y: number; channel: Pick<Channel, 'id' | 'isFavourite'>; extra?: ReactNode } | null;
  show: (e: { clientX: number; clientY: number }, channel: Pick<Channel, 'id' | 'isFavourite'>, extra?: ReactNode) => void;
  close: () => void;
}

export const useChannelMenuStore = create<ChannelMenuState>((set) => ({
  open: null,
  show: (e, channel, extra) => set({ open: { x: e.clientX, y: e.clientY, channel, extra } }),
  close: () => set({ open: null }),
}));

/** Right-click handler for anything that represents a channel. */
export function openChannelMenu(e: React.MouseEvent, channel: Pick<Channel, 'id' | 'isFavourite'>, extra?: ReactNode): void {
  e.preventDefault();
  e.stopPropagation();
  useChannelMenuStore.getState().show(e, channel, extra);
}

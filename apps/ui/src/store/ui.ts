import { create } from 'zustand';
import type { DensitySetting } from '../api/types';

const DENSITY_KEY = 'dl.density';
const RAIL_KEY = 'dl.rail';

function readStored<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return (allowed as readonly string[]).includes(v ?? '') ? (v as T) : fallback;
  } catch {
    return fallback;
  }
}
function writeStored(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* storage unavailable: setting is session-only */
  }
}

/** Writes the density to <html data-density> (CSS keys off it, including the tv button size). */
export function applyDensity(density: DensitySetting): void {
  document.documentElement.dataset.density = density;
}

export type RailPref = 'auto' | 'expanded' | 'collapsed';

interface UiState {
  /** Window fullscreen (the rail hides on the player route). */
  fullscreen: boolean;
  setFullscreen: (f: boolean) => void;
  density: DensitySetting;
  setDensity: (d: DensitySetting) => void;
  /** User preference. 'auto' = collapsed on Guide/Player, expanded elsewhere (DESIGN §8). */
  railPref: RailPref;
  setRailPref: (p: RailPref) => void;
}

export const useUiStore = create<UiState>((set) => ({
  fullscreen: false,
  setFullscreen: (fullscreen) => set({ fullscreen }),
  density: readStored(DENSITY_KEY, ['compact', 'comfortable', 'tv'], 'comfortable'),
  setDensity: (density) => {
    writeStored(DENSITY_KEY, density);
    applyDensity(density);
    set({ density });
  },
  railPref: readStored(RAIL_KEY, ['auto', 'expanded', 'collapsed'], 'auto'),
  setRailPref: (railPref) => {
    writeStored(RAIL_KEY, railPref);
    set({ railPref });
  },
}));

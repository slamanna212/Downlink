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
function hasStored(key: string): boolean {
  try {
    return localStorage.getItem(key) !== null;
  } catch {
    return false;
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
  /** True once the user picked a density; auto-detection then stays out of the way. */
  densityExplicit: boolean;
  /** Apply an auto-detected density without persisting it. */
  setAutoDensity: (d: DensitySetting) => void;
  /** User preference. 'auto' = expanded on every route (DESIGN §8). */
  railPref: RailPref;
  setRailPref: (p: RailPref) => void;
  /** The window is in mini player mode (compact, always on top; PLAN §8.2). */
  mini: boolean;
  setMini: (m: boolean) => void;
  /** A mini player outside the player route is showing video: the shell stops painting so it shows through. */
  videoHole: boolean;
  setVideoHole: (on: boolean) => void;
  /** Last non-video route the user was on: names the player's back pill ("Guide", "Home"…). */
  lastAppPath: string | null;
  setLastAppPath: (p: string) => void;
  /** The first-sync screen is up (DESIGN §12): the shell keeps the rail and status banner out of it. */
  firstSync: boolean;
  setFirstSync: (on: boolean) => void;
}

export const useUiStore = create<UiState>((set) => ({
  fullscreen: false,
  setFullscreen: (fullscreen) => set({ fullscreen }),
  density: readStored(DENSITY_KEY, ['compact', 'comfortable', 'tv'], 'comfortable'),
  densityExplicit: hasStored(DENSITY_KEY),
  setDensity: (density) => {
    writeStored(DENSITY_KEY, density);
    applyDensity(density);
    set({ density, densityExplicit: true });
  },
  setAutoDensity: (density) => {
    applyDensity(density);
    set({ density });
  },
  railPref: readStored(RAIL_KEY, ['auto', 'expanded', 'collapsed'], 'auto'),
  setRailPref: (railPref) => {
    writeStored(RAIL_KEY, railPref);
    set({ railPref });
  },
  mini: false,
  setMini: (mini) => set({ mini }),
  videoHole: false,
  setVideoHole: (videoHole) => set({ videoHole }),
  lastAppPath: null,
  setLastAppPath: (lastAppPath) => set({ lastAppPath }),
  firstSync: false,
  setFirstSync: (firstSync) => set({ firstSync }),
}));

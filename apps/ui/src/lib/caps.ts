import { Cap, type Caps, type Source } from '../api/types';

export function decodeCaps(bits: number): Caps {
  return {
    live: (bits & Cap.Live) !== 0,
    epg: (bits & Cap.Epg) !== 0,
    catchup: (bits & Cap.Catchup) !== 0,
    vod: (bits & Cap.Vod) !== 0,
    dvr: (bits & Cap.Dvr) !== 0,
    favourites: (bits & Cap.Favourites) !== 0,
    streamSwitch: (bits & Cap.StreamSwitch) !== 0,
  };
}

/** Union across all configured sources: a feature exists if any source has the cap. */
export function unionCaps(sources: readonly Source[]): Caps {
  return decodeCaps(sources.reduce((acc, s) => acc | s.caps, 0));
}

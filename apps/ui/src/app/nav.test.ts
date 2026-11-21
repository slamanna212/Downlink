import { describe, expect, it } from 'vitest';
import { decodeCaps } from '../lib/caps';
import { Cap } from '../api/types';
import { visibleNav } from './nav';

const ids = (bits: number) => visibleNav(decodeCaps(bits)).map((e) => e.id);

describe('rail nav', () => {
  it('hides VOD and DVR entries without the caps', () => {
    expect(ids(Cap.Live | Cap.Epg)).toEqual(['search', 'home', 'channels', 'guide']);
  });
  it('shows Movies/Series with vod and Recordings with dvr, in order', () => {
    expect(ids(Cap.Vod)).toEqual(['search', 'home', 'channels', 'guide', 'movies', 'series']);
    expect(ids(Cap.Vod | Cap.Dvr)).toEqual(['search', 'home', 'channels', 'guide', 'movies', 'series', 'recordings']);
    expect(ids(Cap.Dvr)).toContain('recordings');
    expect(ids(Cap.Dvr)).not.toContain('movies');
  });
});

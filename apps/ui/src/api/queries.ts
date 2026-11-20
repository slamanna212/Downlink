// TanStack Query over invoke. Every catalog query key starts with 'catalog' so one
// `catalog://updated` event invalidates them all (see startCatalogBridge).
import { QueryClient, useQuery } from '@tanstack/react-query';
import { invoke, listen } from './ipc';
import type { Caps } from './types';
import { notifyError } from '../components/Notify/notify';
import { unionCaps } from '../lib/caps';

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { staleTime: 30_000, refetchOnWindowFocus: false, retry: 1 },
      mutations: { onError: notifyError },
    },
  });
}

export async function startCatalogBridge(qc: QueryClient): Promise<() => void> {
  return listen('catalog://updated', () => {
    void qc.invalidateQueries({ queryKey: ['catalog'] });
  });
}

export const useSources = () =>
  useQuery({ queryKey: ['catalog', 'sources'], queryFn: () => invoke('list_sources', {}) });

export const useGroups = (sourceId?: number | null) =>
  useQuery({ queryKey: ['catalog', 'groups', sourceId ?? null], queryFn: () => invoke('list_groups', { sourceId }) });

export const useChannels = (args: { groupId?: number | null; query?: string | null; favouritesOnly?: boolean } = {}) =>
  useQuery({ queryKey: ['catalog', 'channels', args], queryFn: () => invoke('list_channels', args) });

export const useNowNext = (channelIds: number[]) =>
  useQuery({
    queryKey: ['catalog', 'nowNext', channelIds],
    queryFn: () => invoke('get_now_next', { channelIds }),
    enabled: channelIds.length > 0,
  });

const NO_CAPS: Caps = { live: false, epg: false, catchup: false, vod: false, dvr: false, favourites: false };

/** UI branches on Caps, never on source kind. `loaded` is false until the first answer. */
export function useCaps(): { caps: Caps; loaded: boolean; hasSource: boolean } {
  const { data, isSuccess } = useSources();
  return { caps: data ? unionCaps(data) : NO_CAPS, loaded: isSuccess, hasSource: (data?.length ?? 0) > 0 };
}

/** Now/next for a window of channels, batched so scrolling does not issue one query per row. */
export function useNowNextWindow(ids: number[], windowKey: string) {
  return useQuery({
    queryKey: ['catalog', 'nowNext', windowKey],
    queryFn: () => invoke('get_now_next', { channelIds: ids }),
    enabled: ids.length > 0,
    placeholderData: (prev) => prev,
    refetchInterval: 60_000,
  });
}

export const useSettings = () =>
  useQuery({ queryKey: ['settings'], queryFn: () => invoke('get_settings', {}), staleTime: Infinity });

export const useCounts = () =>
  useQuery({ queryKey: ['catalog', 'counts'], queryFn: () => invoke('get_counts', {}) });

export const useHome = () =>
  useQuery({ queryKey: ['catalog', 'home'], queryFn: () => invoke('get_home', {}), refetchInterval: 60_000 });

/** True while the (first) source is offline: actions that must reach the server are disabled (PLAN §7.7). */
export function useOffline(): boolean {
  const { data } = useSources();
  return data?.[0]?.status === 'offline';
}

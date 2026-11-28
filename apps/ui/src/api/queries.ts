import { QueryClient, useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect } from 'react';
import { invoke, listen } from './ipc';
import type { Caps, CatalogUpdated, CommandCenterSave, MediaTarget, SportsFollow } from './types';
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

/**
 * Catalog lists only change when the backend says so (`catalog://updated`) or after one of our own
 * mutations, both of which invalidate them; time-based refetching would only repeat the same read.
 */
const EVENT_DRIVEN = 5 * 60_000;

// Sports matches are re-made from guide and channel data, so anything that changes those refreshes them too.
const GUIDE_QUERIES = ['guideProgrammes', 'nowNext', 'programmes', 'home', 'search', 'palette', 'sportsMatches'];
// Saved command centers carry channel DTOs and drop hidden channels, so channel changes refresh them.
const CHANNEL_QUERIES = ['sources', 'channels', 'channel', 'groups', 'counts', 'commandCenters', 'commandCenter', ...GUIDE_QUERIES];
const FAVOURITE_QUERIES = ['channels', 'channel', 'counts', 'home', 'search', 'palette'];
const RECORDING_QUERIES = ['recordings', 'recordingRules', ...GUIDE_QUERIES];

function invalidateCatalog(qc: QueryClient, keys?: string[]) {
  return qc.invalidateQueries({
    queryKey: ['catalog'],
    predicate: keys ? ({ queryKey }) => keys.includes(String(queryKey[1])) : undefined,
  });
}

export const invalidateFavourites = (qc: QueryClient) => invalidateCatalog(qc, FAVOURITE_QUERIES);
export const invalidateRecordings = (qc: QueryClient) => invalidateCatalog(qc, RECORDING_QUERIES);

/** DVR list (upcoming / recording / recorded). Polled every 30 s so "recording now" stays current. */
export function useRecordings(enabled = true) {
  return useQuery({ queryKey: ['catalog', 'recordings'], queryFn: () => invoke('list_recordings', {}), refetchInterval: 30_000, enabled });
}

/** How many recordings are running now (the rail's red record dot). Shares the list query above. */
export function useRecordingsNow(enabled: boolean): number {
  return useRecordings(enabled).data?.inProgress.length ?? 0;
}

const CATALOG_CHANGES: Record<NonNullable<CatalogUpdated['scope']>, string[] | undefined> = {
  status: ['sources'],
  channels: CHANNEL_QUERIES,
  guide: ['sources', ...GUIDE_QUERIES],
  recordings: RECORDING_QUERIES,
  syncComplete: [...FAVOURITE_QUERIES, 'sources', 'groups', ...GUIDE_QUERIES],
  sources: undefined,
  sports: ['sportsMatches'],
};

export async function startCatalogBridge(qc: QueryClient): Promise<() => void> {
  return listen('catalog://updated', ({ scope }) => {
    void invalidateCatalog(qc, scope ? CATALOG_CHANGES[scope] : undefined);
  });
}

/** Keeps the update status query current from `update://status` (rail dot, Settings → Updates). */
export async function startUpdateBridge(qc: QueryClient): Promise<() => void> {
  return listen('update://status', (s) => qc.setQueryData(['update'], s));
}

export const useUpdateStatus = () =>
  useQuery({ queryKey: ['update'], queryFn: () => invoke('get_update_status', {}), staleTime: Infinity });

/** Release notes for a version: the GitHub release body, falling back to latest.json notes (done in Rust). */
export const useReleaseNotes = (version: string | null | undefined) =>
  useQuery({
    queryKey: ['release-notes', version],
    queryFn: () => invoke('get_release_notes', { version: version ?? '' }),
    enabled: !!version,
    staleTime: 10 * 60_000,
  });

/** True while a newer version is waiting (found, downloading or downloaded). */
export function useUpdateWaiting(): boolean {
  return !!useUpdateStatus().data?.available;
}

export const useSources = () =>
  useQuery({ queryKey: ['catalog', 'sources'], queryFn: () => invoke('list_sources', {}), staleTime: EVENT_DRIVEN });

export const useGroups = (sourceId?: number | null) =>
  useQuery({ queryKey: ['catalog', 'groups', sourceId ?? null], queryFn: () => invoke('list_groups', { sourceId }), staleTime: EVENT_DRIVEN });

export const channelQuery = (id: number | null) => ({
  queryKey: ['catalog', 'channel', id],
  queryFn: () => invoke('get_channel', { channelId: id! }),
  enabled: id != null,
  staleTime: EVENT_DRIVEN,
});

export const useChannel = (id: number | null) => useQuery(channelQuery(id));

export function useChannelPages(
  args: { groupId?: number | null; query?: string | null; favouritesOnly?: boolean },
  limit: number,
  enabled = true,
) {
  return useInfiniteQuery({
    queryKey: ['catalog', 'channels', 'pages', args, limit],
    initialPageParam: 0,
    queryFn: ({ pageParam }) => invoke('list_channels', { ...args, offset: pageParam, limit }),
    getNextPageParam: (last, _pages, offset) => last.length === limit ? offset + limit : undefined,
    enabled,
    staleTime: EVENT_DRIVEN,
  });
}

export const useNowNext = (channelIds: number[]) =>
  useQuery({
    queryKey: ['catalog', 'nowNext', channelIds],
    queryFn: () => invoke('get_now_next', { channelIds }),
    enabled: channelIds.length > 0,
  });

const NO_CAPS: Caps = { live: false, epg: false, catchup: false, vod: false, dvr: false, favourites: false, streamSwitch: false };

/** UI branches on Caps, never on source kind. `loaded` is false until the first answer. */
export function useCaps(): { caps: Caps; loaded: boolean; hasSource: boolean } {
  const { data, isSuccess } = useSources();
  return { caps: data ? unionCaps(data) : NO_CAPS, loaded: isSuccess, hasSource: (data?.length ?? 0) > 0 };
}

/** Now/next for a window of channels, batched so scrolling does not issue one query per row. */
export function useNowNextWindow(ids: number[], windowKey: string) {
  return useQuery({
    queryKey: ['catalog', 'nowNext', windowKey, ids],
    queryFn: () => invoke('get_now_next', { channelIds: ids }),
    enabled: ids.length > 0,
    placeholderData: (prev) => prev,
    refetchInterval: 60_000,
  });
}

/** Guide entries for one channel in [start, end) (player timeline). Pass bucketed bounds so the key is stable. */
export function useChannelProgrammes(channelId: number | null, start: number, end: number) {
  return useQuery({
    queryKey: ['catalog', 'programmes', channelId, start, end],
    queryFn: () => invoke('get_programmes', { channelId: channelId ?? 0, start, end }),
    enabled: channelId != null,
    placeholderData: (prev) => prev,
  });
}

export const useSettings = () =>
  useQuery({ queryKey: ['settings'], queryFn: () => invoke('get_settings', {}), staleTime: Infinity });

export const useCounts = () =>
  useQuery({ queryKey: ['catalog', 'counts'], queryFn: () => invoke('get_counts', {}), staleTime: EVENT_DRIVEN });

export const useHome = () =>
  useQuery({ queryKey: ['catalog', 'home'], queryFn: () => invoke('get_home', {}), refetchInterval: 60_000 });

/** True while the (first) source is offline: actions that must reach the server are disabled (PLAN §7.7). */
export function useOffline(): boolean {
  const { data } = useSources();
  return data?.[0]?.status === 'offline';
}

// ---- media server (Plex) ----------------------------------------------------------------------

export function useMediaServerStatus() {
  return useQuery({ queryKey: ['mediaServer', 'status'], queryFn: () => invoke('media_server_status', {}) });
}

/**
 * Artwork for a guide programme. Core picks and caches the source (guide icon, server, TMDB), so
 * this is cheap after the first view; null means "show the gradient".
 */
export function useProgrammeArt(programmeId: string | null | undefined) {
  return useQuery({
    queryKey: ['programmeArt', programmeId],
    queryFn: () => invoke('programme_art', { id: programmeId as string }),
    enabled: !!programmeId,
    staleTime: Infinity,
    gcTime: 30 * 60_000,
    retry: false,
  });
}

/** The playing title on the user's media server; idle until a server is chosen. Failures just hide the button. */
export function useMediaMatch(target: MediaTarget | null) {
  const status = useMediaServerStatus();
  return useQuery({
    queryKey: ['mediaServer', 'match', target],
    queryFn: () => invoke('media_server_match', { target: target as MediaTarget }),
    enabled: !!target && !!status.data?.plex.serverId,
    staleTime: 5 * 60_000,
    retry: false,
  });
}

// ---- sports tracking (docs/SPORTS.md) ---------------------------------------------------------

/** The embedded catalog only changes with an app update. */
export const useSportsCatalog = () =>
  useQuery({ queryKey: ['sports', 'catalog'], queryFn: () => invoke('sports_catalog', {}), staleTime: Infinity });

const SPORTS_FOLLOW_KEY = ['sports', 'follow'];
const SPORTS_FOLLOW_SET = ['sports', 'follow', 'set'];

export const useSportsFollow = () =>
  useQuery({ queryKey: SPORTS_FOLLOW_KEY, queryFn: () => invoke('sports_follow_get', {}), staleTime: Infinity });

/**
 * Saves the whole follow document, optimistically: switches and chips flip at once and roll back on
 * error. The backend re-matches and emits `catalog://updated` (scope `sports`), which refreshes matches.
 */
export function useSetSportsFollow() {
  const qc = useQueryClient();
  return useMutation({
    mutationKey: SPORTS_FOLLOW_SET,
    mutationFn: (follow: SportsFollow) => invoke('sports_follow_set', { follow }),
    onMutate: async (follow) => {
      await qc.cancelQueries({ queryKey: SPORTS_FOLLOW_KEY, exact: true });
      const previous = qc.getQueryData<SportsFollow>(SPORTS_FOLLOW_KEY);
      qc.setQueryData(SPORTS_FOLLOW_KEY, follow);
      return { previous };
    },
    onError: (e, _follow, ctx) => {
      if (ctx?.previous) qc.setQueryData(SPORTS_FOLLOW_KEY, ctx.previous);
      notifyError(e);
    },
    // Re-read only after the last of a burst of quick toggles, so an early answer never undoes a later click.
    onSettled: () => {
      if (qc.isMutating({ mutationKey: SPORTS_FOLLOW_SET }) <= 1) void qc.invalidateQueries({ queryKey: SPORTS_FOLLOW_KEY, exact: true });
    },
  });
}

/** Matches in [startUtc, endUtc). Pass bucketed bounds so the key is stable. Refetched each minute for live state. */
export function useSportsMatches(startUtc: number, endUtc: number, enabled = true) {
  return useQuery({
    queryKey: ['catalog', 'sportsMatches', startUtc, endUtc],
    queryFn: () => invoke('sports_matches', { startUtc, endUtc }),
    enabled,
    placeholderData: (prev) => prev,
    refetchInterval: 60_000,
  });
}

// ---- saved command centers (COMMAND_CENTER §6.1) -------------------------------------------------

export const COMMAND_CENTERS_KEY = ['catalog', 'commandCenters'];

/** Saved centers, most recently changed first. */
export const useCommandCenters = () =>
  useQuery({ queryKey: COMMAND_CENTERS_KEY, queryFn: () => invoke('command_centers_list', {}), staleTime: EVENT_DRIVEN });

export const commandCenterQuery = (id: number | null) => ({
  queryKey: ['catalog', 'commandCenter', id],
  queryFn: () => invoke('command_center_get', { id: id! }),
  enabled: id != null,
  staleTime: EVENT_DRIVEN,
  retry: false,
});

export const useCommandCenter = (id: number | null) => useQuery(commandCenterQuery(id));

function invalidateCenters(qc: QueryClient) {
  return invalidateCatalog(qc, ['commandCenters', 'commandCenter']);
}

/** Creates (`id` null) or replaces a center; resolves to its id. */
export function useSaveCommandCenter() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (center: CommandCenterSave) => invoke('command_center_save', { center }),
    onSuccess: () => invalidateCenters(qc),
  });
}

export function useDeleteCommandCenter() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => invoke('command_center_delete', { id }),
    onSuccess: (_r, id) => {
      qc.removeQueries({ queryKey: ['catalog', 'commandCenter', id] });
      return invalidateCenters(qc);
    },
  });
}

// ---- news feeds (COMMAND_CENTER §6.2) ------------------------------------------------------------
// Not under 'catalog': guide/channel syncs never touch feeds. `feeds://updated` and our own mutations
// invalidate them.

export const FEEDS_KEY = ['feeds'] as const;
const FEED_LIST_KEY = ['feeds', 'list'] as const;

/** All added feeds, sorted by title. */
export const useFeeds = () =>
  useQuery({ queryKey: FEED_LIST_KEY, queryFn: () => invoke('feeds_list', {}), staleTime: EVENT_DRIVEN });

/** Headlines of `feedIds`, newest first (disabled while the list is empty). */
export function useFeedItems(feedIds: readonly number[], limit = 100) {
  const ids = [...new Set(feedIds)].sort((a, b) => a - b);
  return useQuery({
    queryKey: ['feeds', 'items', ids, limit],
    queryFn: () => invoke('feed_items', { feedIds: ids, limit }),
    enabled: ids.length > 0,
    placeholderData: (prev) => prev,
    staleTime: EVENT_DRIVEN,
  });
}

const invalidateFeeds = (qc: QueryClient) => qc.invalidateQueries({ queryKey: FEEDS_KEY });

/** Validates and fetches the feed once in Rust; rejects (nothing saved) when that fetch fails. */
export function useAddFeed() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: (url: string) => invoke('feed_add', { url }), onSuccess: () => invalidateFeeds(qc) });
}

export function useRemoveFeed() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: (id: number) => invoke('feed_remove', { id }), onSuccess: () => invalidateFeeds(qc) });
}

/** Manual refresh. A failure still updates the feed's `lastError`, so the list is re-read either way. */
export function useRefreshFeed() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: (id: number) => invoke('feed_refresh', { id }), onSettled: () => invalidateFeeds(qc) });
}

let watchSeq = 0;

/**
 * Polls `feedIds` in Rust while the calling view is mounted (call it once per open command center):
 * `feeds_watch` on mount and whenever the set changes, `feeds_unwatch` on unmount. Every effect run
 * uses a fresh key, so a late unwatch from an earlier run never cancels a newer watch. Invalidates
 * feed queries on `feeds://updated` for these feeds.
 */
export function useFeedsWatch(feedIds: readonly number[]): void {
  const qc = useQueryClient();
  const idsKey = [...new Set(feedIds)].sort((a, b) => a - b).join(',');

  useEffect(() => {
    const watched = idsKey ? idsKey.split(',').map(Number) : [];
    if (watched.length === 0) return;
    const key = `watch-${++watchSeq}-${Math.random().toString(36).slice(2, 8)}`;
    void invoke('feeds_watch', { key, feedIds: watched }).catch(() => {});
    let unlisten: (() => void) | null = null;
    let live = true;
    void listen('feeds://updated', ({ feedIds: changed }) => {
      if (changed.some((id) => watched.includes(id))) void invalidateFeeds(qc);
    }).then((u) => {
      if (live) unlisten = u;
      else u();
    });
    return () => {
      live = false;
      unlisten?.();
      void invoke('feeds_unwatch', { key }).catch(() => {});
    };
  }, [idsKey, qc]);
}

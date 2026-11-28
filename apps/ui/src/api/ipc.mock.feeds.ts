// News feeds for the mock backend (COMMAND_CENTER §6.2). SYNTHETIC data only (CLAUDE rule 16): invented
// feeds on `.example` hosts and invented headlines. While any view watches a feed, a fresh headline
// lands every ~30 s and `feeds://updated` fires, like the Rust poller would.
import type { Commands, Events } from './ipc';
import type { AppError, Feed, FeedItem } from './types';

type FeedCommands = Pick<
  Commands,
  'feeds_list' | 'feed_add' | 'feed_remove' | 'feed_items' | 'feed_refresh' | 'feeds_watch' | 'feeds_unwatch'
>;
export type MockFeedImpl = {
  [C in keyof FeedCommands]: (a: FeedCommands[C]['args']) => FeedCommands[C]['result'] | Promise<FeedCommands[C]['result']>;
};

const nowSec = () => Math.floor(Date.now() / 1000);
const err = (code: AppError['code'], message: string): AppError => ({ code, message, retryAfter: null });

const HEADLINES = [
  'Harbor bridge reopens after overnight repairs',
  'City council approves new bike lanes downtown',
  'Regional rail adds late-night weekend service',
  'Storm system expected to clear by morning',
  'Local orchestra announces winter concert series',
  'Library extends hours for exam season',
  'Tech startup opens second office on the waterfront',
  'Farmers market moves indoors for the season',
  'Museum unveils restored lighthouse lens',
  'Ferry schedule changes take effect Monday',
  'University team wins robotics championship',
  'Water main work to close two blocks this week',
];

const SUMMARY = 'An invented summary for the mock backend. Plain text only, already trimmed to a few lines so the rail can show it as is.';

/** Credential-free display form, like core's: no userinfo, credential query values masked. */
function display(raw: string): { url: string; hasSecret: boolean } {
  const u = new URL(raw);
  let hasSecret = u.username !== '' || u.password !== '';
  u.username = '';
  u.password = '';
  u.hash = '';
  for (const k of [...u.searchParams.keys()]) {
    if (/^(token|key|apikey|api_key|api-key|access_token|auth|auth_token|secret|sig|signature|password|passkey)$/i.test(k)) {
      u.searchParams.set(k, '***');
      hasSecret = true;
    }
  }
  return { url: hasSecret ? u.toString().replace(/%2A%2A%2A/g, '***') : raw.trim().replace(/#.*$/, ''), hasSecret };
}

export function createMockFeeds(emit: <E extends 'feeds://updated'>(event: E, payload: Events[E]) => void, seed: boolean): MockFeedImpl {
  const feeds: Feed[] = [];
  const items = new Map<number, FeedItem[]>();
  const watches = new Map<string, Set<number>>();
  let nextId = 1;
  let counter = 0;
  let timer: ReturnType<typeof setInterval> | null = null;

  const headline = (feedId: number, at: number): FeedItem => {
    const n = counter++;
    const host = feeds.find((f) => f.id === feedId)?.siteUrl ?? 'https://news.example/';
    return {
      feedId,
      guid: `mock-${feedId}-${n}`,
      title: HEADLINES[n % HEADLINES.length]!,
      link: `${host}story/${n}`,
      summary: n % 3 === 2 ? null : SUMMARY,
      publishedUtc: at,
    };
  };
  const touch = (f: Feed) => {
    const t = nowSec();
    f.lastFetchUtc = t;
    f.lastOkUtc = t;
    f.lastError = null;
    f.itemCount = items.get(f.id)?.length ?? 0;
  };
  const add = (raw: string, title: string, site: string, count: number): Feed => {
    const { url, hasSecret } = display(raw);
    const f: Feed = { id: nextId++, url, hasSecret, title, siteUrl: site, lastFetchUtc: null, lastOkUtc: null, lastError: null, itemCount: 0 };
    feeds.push(f);
    const now = nowSec();
    items.set(f.id, Array.from({ length: count }, (_, i) => headline(f.id, now - (i + 1) * 17 * 60)));
    touch(f);
    return f;
  };
  const watched = () => new Set([...watches.values()].flatMap((s) => [...s]));
  const sync = () => {
    const any = watched().size > 0;
    if (any && !timer) {
      timer = setInterval(() => {
        const ids = [...watched()].filter((id) => feeds.some((f) => f.id === id));
        if (ids.length === 0) return;
        const id = ids[counter % ids.length]!;
        const list = items.get(id) ?? [];
        list.unshift(headline(id, nowSec()));
        items.set(id, list.slice(0, 200));
        const f = feeds.find((x) => x.id === id);
        if (f) touch(f);
        emit('feeds://updated', { feedIds: [id] });
      }, 30_000);
    } else if (!any && timer) {
      clearInterval(timer);
      timer = null;
    }
  };

  if (seed) {
    add('https://news.example/rss.xml', 'Example Daily', 'https://news.example/', 8);
    add('https://wire.example/feed.atom', 'Sample Wire', 'https://wire.example/', 6);
    const broken = add('https://broken.example/rss', 'Offline Gazette', 'https://broken.example/', 2);
    broken.lastOkUtc = nowSec() - 3 * 3600;
    broken.lastError = 'The server had a problem (503).';
  }

  return {
    feeds_list: () =>
      [...feeds].sort((a, b) => (a.title ?? a.url).localeCompare(b.title ?? b.url) || a.id - b.id).map((f) => ({ ...f })),
    feed_add: async ({ url }) => {
      await new Promise((r) => setTimeout(r, 400));
      const raw = url.trim();
      let parsed: URL;
      try {
        parsed = new URL(raw);
      } catch {
        throw err('local', 'That feed address is not valid.');
      }
      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') throw err('local', 'Feed addresses must start with http:// or https://.');
      if (parsed.hostname.includes('fail')) throw err('server', 'That address did not return an RSS or Atom feed.');
      if (feeds.some((f) => f.url === display(raw).url)) throw err('local', 'That feed is already added.');
      const f = add(raw, `Feed from ${parsed.hostname}`, `${parsed.protocol}//${parsed.host}/`, 5);
      emit('feeds://updated', { feedIds: [f.id] });
      return { ...f };
    },
    feed_remove: ({ id }) => {
      const i = feeds.findIndex((f) => f.id === id);
      if (i >= 0) feeds.splice(i, 1);
      items.delete(id);
      for (const s of watches.values()) s.delete(id);
      sync();
      return null;
    },
    feed_items: ({ feedIds, limit }) => {
      const want = new Set(feedIds);
      return [...items.entries()]
        .filter(([id]) => want.has(id))
        .flatMap(([, list]) => list)
        .sort((a, b) => b.publishedUtc - a.publishedUtc || a.feedId - b.feedId)
        .slice(0, Math.max(1, Math.min(500, limit)))
        .map((i) => ({ ...i }));
    },
    feed_refresh: async ({ id }) => {
      const f = feeds.find((x) => x.id === id);
      if (!f) throw err('local', 'That feed no longer exists.');
      await new Promise((r) => setTimeout(r, 300));
      if (f.url.includes('broken.example')) {
        f.lastFetchUtc = nowSec();
        throw err('server', f.lastError ?? 'The server had a problem (503).');
      }
      const list = items.get(id) ?? [];
      list.unshift(headline(id, nowSec()));
      items.set(id, list);
      touch(f);
      emit('feeds://updated', { feedIds: [id] });
      return { ...f };
    },
    feeds_watch: ({ key, feedIds }) => {
      if (feedIds.length === 0) watches.delete(key);
      else watches.set(key, new Set(feedIds));
      sync();
      return null;
    },
    feeds_unwatch: ({ key }) => {
      watches.delete(key);
      sync();
      return null;
    },
  };
}

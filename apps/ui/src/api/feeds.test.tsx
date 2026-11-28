import { QueryClientProvider } from '@tanstack/react-query';
import { act, renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { setBackend, type Backend, type EventName } from './ipc';
import { createMockFeeds } from './ipc.mock.feeds';
import { createQueryClient, useFeedItems, useFeedsWatch } from './queries';
import type { FeedsUpdated } from './types';

function recordingBackend() {
  const calls: { cmd: string; args: unknown }[] = [];
  const handlers = new Map<EventName, Set<(p: never) => void>>();
  const backend: Backend = {
    async invoke(cmd, args) {
      calls.push({ cmd, args });
      if (cmd === 'feed_items') return [] as never;
      return null as never;
    },
    async listen(event, handler) {
      const set = handlers.get(event) ?? new Set();
      set.add(handler as (p: never) => void);
      handlers.set(event, set);
      return () => set.delete(handler as (p: never) => void);
    },
  };
  const emit = (p: FeedsUpdated) => handlers.get('feeds://updated')?.forEach((h) => h(p as never));
  return { backend, calls, emit, listeners: () => handlers.get('feeds://updated')?.size ?? 0 };
}

afterEach(() => {
  setBackend(null);
  vi.useRealTimers();
});

describe('useFeedsWatch', () => {
  it('watches on mount, re-watches on change, unwatches on unmount and refetches on feeds://updated', async () => {
    const rb = recordingBackend();
    setBackend(rb.backend);
    const qc = createQueryClient();
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
    const { rerender, unmount } = renderHook(
      ({ ids }: { ids: number[] }) => {
        useFeedsWatch(ids);
        return useFeedItems(ids);
      },
      { wrapper, initialProps: { ids: [2, 1, 2] } },
    );
    await waitFor(() => expect(rb.listeners()).toBe(1));
    const watches = () => rb.calls.filter((c) => c.cmd === 'feeds_watch').map((c) => c.args as { key: string; feedIds: number[] });
    expect(watches()).toHaveLength(1);
    expect(watches()[0]!.feedIds).toEqual([1, 2]);
    const firstKey = watches()[0]!.key;

    // Same set in another order: no new watch.
    rerender({ ids: [1, 2] });
    expect(watches()).toHaveLength(1);

    const itemReads = () => rb.calls.filter((c) => c.cmd === 'feed_items').length;
    await waitFor(() => expect(itemReads()).toBe(1));
    act(() => rb.emit({ feedIds: [9] }));
    act(() => rb.emit({ feedIds: [2] }));
    await waitFor(() => expect(itemReads()).toBe(2));

    rerender({ ids: [3] });
    await waitFor(() => expect(watches()).toHaveLength(2));
    const unwatches = () => rb.calls.filter((c) => c.cmd === 'feeds_unwatch').map((c) => (c.args as { key: string }).key);
    await waitFor(() => expect(unwatches()).toEqual([firstKey]));
    expect(watches()[1]!.key).not.toBe(firstKey);

    unmount();
    await waitFor(() => expect(unwatches()).toEqual([firstKey, watches()[1]!.key]));
    await waitFor(() => expect(rb.listeners()).toBe(0));
  });

  it('does nothing for an empty list', async () => {
    const rb = recordingBackend();
    setBackend(rb.backend);
    const qc = createQueryClient();
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
    const { unmount } = renderHook(() => useFeedsWatch([]), { wrapper });
    unmount();
    await new Promise((r) => setTimeout(r, 10));
    expect(rb.calls).toEqual([]);
  });
});

describe('mock feeds', () => {
  it('adds a fresh headline every 30 s while watched and masks credential URLs', async () => {
    vi.useFakeTimers();
    const events: FeedsUpdated[] = [];
    const m = createMockFeeds((_e, p) => events.push(p), true);
    const feeds = await m.feeds_list({});
    expect(feeds.length).toBe(3);
    const id = feeds[0]!.id;
    const before = (await m.feed_items({ feedIds: [id], limit: 500 })).length;

    vi.advanceTimersByTime(31_000);
    expect(events).toEqual([]);
    m.feeds_watch({ key: 'a', feedIds: [id] });
    vi.advanceTimersByTime(30_000);
    expect(events).toEqual([{ feedIds: [id] }]);
    const after = await m.feed_items({ feedIds: [id], limit: 500 });
    expect(after.length).toBe(before + 1);
    expect(after[0]!.publishedUtc).toBeGreaterThanOrEqual(after[1]!.publishedUtc);
    m.feeds_unwatch({ key: 'a' });
    vi.advanceTimersByTime(60_000);
    expect(events).toHaveLength(1);

    const added = m.feed_add({ url: 'https://reader:pw@private.example/rss?token=abc123&lang=en' });
    vi.advanceTimersByTime(500);
    const f = await added;
    expect(f.hasSecret).toBe(true);
    expect(f.url).not.toContain('abc123');
    expect(f.url).not.toContain('pw@');
    expect(f.url).toContain('lang=en');
  });
});

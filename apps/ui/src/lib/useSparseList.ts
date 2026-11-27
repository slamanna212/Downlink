import { useQueries, useQueryClient, type QueryKey } from '@tanstack/react-query';

/** One page as a list command returns it; `total` when the backend knows the full count. */
export interface SparsePage<T> {
  items: T[];
  total?: number | null;
}

export interface SparseListOptions<T> {
  /** Prefix for the per-page query keys (pages are `[...queryKey, 'page', pageSize, n]`). */
  queryKey: QueryKey;
  pageSize: number;
  fetchPage: (offset: number, limit: number) => Promise<SparsePage<T>>;
  /** First and last row index on screen (inclusive); only the pages covering them are fetched. */
  start: number;
  end: number;
  /** Known row count, when the caller already has it (e.g. a group's channel count). */
  total?: number | null;
  enabled?: boolean;
  staleTime?: number;
}

export interface SparseList<T> {
  /** Rows to lay out: the known total, else everything loaded so far plus one page when more may follow. */
  count: number;
  /** True when `count` is the real total (not a lower bound). */
  exact: boolean;
  /** The row at `i`, once its page has loaded. */
  get: (i: number) => T | undefined;
  /** The first page has not arrived yet. */
  isLoading: boolean;
  /** The first page failed and nothing is shown. */
  isError: boolean;
  /** Some page on screen failed. */
  pageError: boolean;
  refetch: () => void;
}

/**
 * Windowed paging for long virtualised lists: the list is laid out at its full size up front and
 * only the pages on screen are requested, so jumping to row 18,000 is one request, not 120, and a
 * catalog refresh refetches the visible pages only. Without a known total it grows page by page
 * like an infinite list.
 */
export function useSparseList<T>(o: SparseListOptions<T>): SparseList<T> {
  const qc = useQueryClient();
  const { pageSize, enabled = true } = o;
  const keyOf = (n: number): QueryKey => [...o.queryKey, 'page', pageSize, n];
  const cached = (n: number) => qc.getQueryData<SparsePage<T>>(keyOf(n));

  // How far the list is known to reach, from the pages already in the cache.
  let reported: number | null = o.total ?? cached(0)?.total ?? null;
  let loadedEnd = 0;
  let shortEnd: number | null = null;
  for (let n = 0; ; n++) {
    const p = cached(n);
    if (!p) break;
    loadedEnd = n * pageSize + p.items.length;
    if (p.items.length < pageSize) {
      shortEnd = loadedEnd;
      break;
    }
  }
  if (reported != null && shortEnd != null) reported = Math.min(reported, shortEnd);
  const exact = reported != null || shortEnd != null;
  const count = reported ?? shortEnd ?? (loadedEnd > 0 ? loadedEnd + pageSize : 0);

  const last = Math.max(0, count - 1);
  const first = Math.min(Math.max(0, o.start), last);
  const lastPage = Math.floor(Math.min(Math.max(o.end, first), last) / pageSize);
  const pages = new Set<number>([0]);
  for (let n = Math.floor(first / pageSize); n <= lastPage; n++) pages.add(n);
  const wanted = [...pages];

  const results = useQueries({
    queries: wanted.map((n) => ({
      queryKey: keyOf(n),
      queryFn: () => o.fetchPage(n * pageSize, pageSize),
      enabled,
      staleTime: o.staleTime,
    })),
  });
  const byPage = new Map<number, SparsePage<T> | undefined>(wanted.map((n, i) => [n, results[i]?.data]));
  const first0 = results[0];

  return {
    count,
    exact,
    get: (i) => {
      if (i < 0) return undefined;
      const n = Math.floor(i / pageSize);
      return (byPage.get(n) ?? cached(n))?.items[i - n * pageSize];
    },
    isLoading: enabled && !first0?.data && !first0?.isError,
    isError: !!first0?.isError && !first0.data,
    pageError: results.some((r) => r.isError),
    refetch: () => results.forEach((r) => r.isError && void r.refetch()),
  };
}

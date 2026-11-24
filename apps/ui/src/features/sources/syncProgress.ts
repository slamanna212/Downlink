import type { TaskProgress } from '../../api/types';

/**
 * Folds `task://progress` events from the first sync of a new source into what the first-sync screen
 * shows. The core emits a marker as each phase starts (connect 0/4, channels 1/4, guide 2/4, state 3/4,
 * done 4/4), then per-phase counts: channels (fetched, total or 0), guide (programmes so far with total 0,
 * or windows done/total). Pure, so it is unit-tested.
 */
export type SyncPhase = 'connect' | 'channels' | 'guide' | 'state' | 'done';

export interface SyncView {
  phase: SyncPhase;
  /** Channels fetched so far, and the total when the source says (0 = unknown). */
  channels: number;
  channelsTotal: number;
  /** Guide progress: programmes written (total 0) or windows done of total. */
  guideDone: number;
  guideTotal: number;
  /** 0..1, never goes backwards. */
  overall: number;
}

export const SYNC_START: SyncView = { phase: 'connect', channels: 0, channelsTotal: 0, guideDone: 0, guideTotal: 0, overall: 0 };

const ORDER: SyncPhase[] = ['connect', 'channels', 'guide', 'state', 'done'];
/** Share of the overall bar each phase owns. The guide is the long one. */
const SPAN: Record<SyncPhase, [number, number]> = {
  connect: [0, 0.05],
  channels: [0.05, 0.4],
  guide: [0.4, 0.9],
  state: [0.9, 0.98],
  done: [1, 1],
};

function isPhase(p: string): p is SyncPhase {
  return (ORDER as string[]).includes(p);
}

/** Unknown totals creep toward the end of the phase instead of jumping. */
function within(done: number, total: number, unknownScale: number): number {
  if (total > 0) return Math.min(1, done / total);
  return 1 - Math.exp(-done / unknownScale);
}

export function reduceSync(prev: SyncView, e: TaskProgress): SyncView {
  if (!isPhase(e.phase)) return prev;
  const at = ORDER.indexOf(e.phase);
  const was = ORDER.indexOf(prev.phase);
  if (at < was) return prev; // a late event from an earlier phase
  const next: SyncView = { ...prev, phase: e.phase };
  // The first event of a phase is the step marker (done/total are step numbers), not counts.
  if (at > was) {
    next.overall = Math.max(prev.overall, SPAN[e.phase][0]);
    return next;
  }
  let f = 0;
  if (e.phase === 'channels') {
    next.channels = e.done;
    next.channelsTotal = e.total;
    f = within(e.done, e.total, 2000);
  } else if (e.phase === 'guide') {
    next.guideDone = e.done;
    next.guideTotal = e.total;
    f = within(e.done, e.total, 20000);
  }
  const [a, b] = SPAN[e.phase];
  next.overall = Math.max(prev.overall, a + (b - a) * f);
  return next;
}

/** The screen's end state once `add_source` has returned. */
export function finishSync(prev: SyncView): SyncView {
  return { ...prev, phase: 'done', overall: 1 };
}

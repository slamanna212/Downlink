import { create } from 'zustand';
import { listen } from '../api/ipc';
import type { TaskProgress } from '../api/types';

interface TasksState {
  /** Latest progress per source id; cleared when done >= total. */
  bySource: Record<number, TaskProgress>;
  set: (p: TaskProgress) => void;
}

export const useTasksStore = create<TasksState>((set) => ({
  bySource: {},
  set: (p) =>
    set((s) => {
      const next = { ...s.bySource };
      if (p.total > 0 && p.done >= p.total) delete next[p.sourceId];
      else next[p.sourceId] = p;
      return { bySource: next };
    }),
}));

export function startTaskBridge(): Promise<() => void> {
  return listen('task://progress', (p) => useTasksStore.getState().set(p));
}

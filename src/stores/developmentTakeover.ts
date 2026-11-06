import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopment } from './developmentStore';
import { contextGeneration } from './developmentSelection';
import { hostCode } from './hostError';

export interface TakeoverView {
  project_id: string; task_id: string; directory: string;
  sessions: { id: string; task_id: string | null; engine: 'codex' | 'claude'; status: string }[];
  restricted: boolean; host_busy: boolean; can_close: boolean; token: string;
}
interface Entry { view: TakeoverView | null; busy: boolean; error: string | null; released: boolean }
interface State {
  entries: Record<string, Entry>;
  refresh(projectId: string, taskId: string): Promise<void>;
  close(projectId: string, taskId: string, reviewed: TakeoverView): Promise<boolean>;
}
export const takeoverKey = (projectId: string, taskId: string) => JSON.stringify([projectId, taskId]);
const revisions = new Map<string, number>();
export const useDevelopmentTakeover = create<State>((set, get) => {
  const update = (key: string, entry: Entry) => set(state => ({ entries: { ...state.entries, [key]: entry } }));
  return {
    entries: {},
    async refresh(projectId, taskId) {
      const key = takeoverKey(projectId, taskId);
      const epoch = useDevelopment.getState().epoch;
      const generation = contextGeneration();
      const revision = (revisions.get(key) ?? 0) + 1; revisions.set(key, revision);
      const current = () => epoch === useDevelopment.getState().epoch && generation === contextGeneration() && revision === revisions.get(key);
      const before = get().entries[key];
      update(key, { view: before?.view ?? null, busy: true, error: null, released: before?.released ?? false });
      try {
        const view = await invoke<TakeoverView>('read_development_takeover', { input: { project_id: projectId, task_id: taskId } });
        if (current()) update(key, { view, busy: false, error: null, released: Boolean(before?.released && !view.sessions.length && !view.restricted && !view.host_busy) });
      } catch (error) {
        if (current()) update(key, { view: null, busy: false, error: hostCode(error), released: false });
      } finally {
        // A context change may leave this cached entry mounted elsewhere. It grants no authority.
        if (!current() && epoch === useDevelopment.getState().epoch && revision === revisions.get(key)) update(key, { view: null, busy: false, error: null, released: false });
      }
    },
    async close(projectId, taskId, reviewed) {
      const key = takeoverKey(projectId, taskId);
      const before = get().entries[key];
      if (before?.busy || before?.view !== reviewed || !reviewed.can_close) return false;
      const epoch = useDevelopment.getState().epoch;
      const generation = contextGeneration();
      const revision = (revisions.get(key) ?? 0) + 1; revisions.set(key, revision);
      const current = () => epoch === useDevelopment.getState().epoch && generation === contextGeneration() && revision === revisions.get(key);
      update(key, { view: reviewed, busy: true, error: null, released: false });
      try {
        await invoke<TakeoverView>('close_development_takeover', { input: { project_id: projectId, task_id: taskId, token: reviewed.token, confirmed: true } });
        if (!current()) return false;
        const view = await invoke<TakeoverView>('read_development_takeover', { input: { project_id: projectId, task_id: taskId } });
        if (!current()) return false;
        const released = !view.sessions.length && !view.restricted && !view.host_busy;
        update(key, { view, busy: false, error: null, released });
        return released;
      } catch (error) {
        if (current()) update(key, { view: null, busy: false, error: hostCode(error), released: false });
        return false;
      } finally {
        if (!current() && epoch === useDevelopment.getState().epoch && revision === revisions.get(key)) update(key, { view: null, busy: false, error: null, released: false });
      }
    },
  };
});
useDevelopment.subscribe((state, previous) => {
  if (state.epoch !== previous.epoch) { revisions.clear(); useDevelopmentTakeover.setState({ entries: {} }); }
});

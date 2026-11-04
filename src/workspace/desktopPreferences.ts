import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
export interface DesktopValues { pinnedModuleIds: string[]; showEntries: boolean; showRecent: boolean; sidebarCollapsed: boolean }
interface Snapshot { revision: number; values: DesktopValues }
interface Response<T> { success: boolean; data: T | null }
const defaults = (): DesktopValues => ({ pinnedModuleIds: [], showEntries: true, showRecent: true, sidebarCollapsed: false });
let epoch = 0;
let change = 0;
interface State {
  values: DesktopValues; revision: number; loaded: boolean; busy: boolean; dirty: boolean; error: 'failed' | 'conflict' | null;
  load: () => Promise<void>; patch: (values: Partial<DesktopValues>) => void; save: () => Promise<void>; reset: () => void;
}
export const useDesktopPreferences = create<State>((set, get) => ({
  values: defaults(), revision: 0, loaded: false, busy: false, dirty: false, error: null,
  async load() {
    if (get().busy || (get().loaded && !get().error)) return;
    const owner = epoch;
    set({ busy: true });
    try {
      const result = await invoke<Response<Snapshot>>('get_desktop_preferences');
      if (owner !== epoch) return;
      if (!result.success || !result.data) throw new Error('unavailable');
      change++;
      set({ ...result.data, loaded: true, busy: false, dirty: false, error: null });
    } catch { if (owner === epoch) set({ busy: false, error: 'failed' }); }
  },
  patch(patch) {
    if (!get().loaded) return;
    const values = { ...get().values, ...patch };
    if (JSON.stringify(values) === JSON.stringify(get().values)) return;
    change++;
    set({ values, dirty: true });
    if (!get().error) void get().save();
  },
  async save() {
    if (!get().loaded || get().busy || !get().dirty || get().error) return;
    const owner = epoch;
    const version = change;
    const { values, revision } = get();
    set({ busy: true, error: null });
    try {
      const result = await invoke<Response<{ kind: 'saved' | 'conflict'; snapshot: Snapshot }>>('set_desktop_preferences', { values, expectedRevision: revision });
      if (owner !== epoch) return;
      if (!result.success || !result.data) throw new Error('unavailable');
      if (result.data.kind === 'conflict') { set({ busy: false, error: 'conflict' }); return; }
      set({ revision: result.data.snapshot.revision, busy: false, dirty: version !== change });
      if (get().dirty) void get().save();
    } catch {
      // The write may have succeeded: only a read can resolve that uncertainty.
      if (owner === epoch) set({ busy: false, error: 'failed' });
    }
  },
  reset() { epoch++; change++; set({ values: defaults(), revision: 0, loaded: false, busy: false, dirty: false, error: null }); },
}));

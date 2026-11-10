import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';

interface DesktopSnapshot { revision: number; value: Record<string, unknown> }
interface DesktopPersistence {
  owner: string | null; loaded: boolean; loading: boolean; saving: boolean;
  dirty: boolean; error: string | null; conflict: boolean;
  value: Record<string, unknown>;
}
/** Private host storage only. This module deliberately imports no development stores. */
export const useDesktopPersistence = create<DesktopPersistence>(() => ({
  owner: null, loaded: false, loading: false, saving: false, dirty: false,
  error: null, conflict: false, value: {},
}));
const restorers = new Set<() => void>();
let lifecycle = 0, revision = 0, changes = 0;
let timer: number | undefined;
let initialization: Promise<void> | null = null;
let flight: Promise<boolean> | null = null;
const edited = new Set<string>();
let applying = false;
let hydrated = false;
let restoringValue: Record<string, unknown> | null = null;
const MAX_STATE_BYTES = 8 * 1024 * 1024;
function failure(error: unknown): string {
  const message = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
  if (message === 'invalid_desktop_state') return 'desktop_state_invalid';
  return ['desktop_state_conflict', 'desktop_state_limit', 'desktop_state_invalid', 'desktop_state_forbidden'].find(code => message.includes(code)) ?? 'desktop_state_save_failed';
}
function valid(snapshot: DesktopSnapshot): boolean {
  return !!snapshot && Number.isSafeInteger(snapshot.revision) && snapshot.revision >= 0 && !!snapshot.value && typeof snapshot.value === 'object' && !Array.isArray(snapshot.value);
}
function restore(value: Record<string, unknown>) {
  applying = true; restoringValue = value;
  try { for (const apply of restorers) apply(); }
  finally { applying = false; restoringValue = null; }
}
export function onDesktopStateRestored(apply: () => void): () => void {
  restorers.add(apply);
  if (useDesktopPersistence.getState().loaded) { applying = true; try { apply(); } finally { applying = false; } }
  return () => { restorers.delete(apply); };
}
export function resetDesktopState(): void {
  ++lifecycle; ++changes;
  clearTimeout(timer); timer = undefined; initialization = null; flight = null;
  hydrated = false;
  edited.clear(); revision = 0;
  useDesktopPersistence.setState({ owner: null, loaded: false, loading: false, saving: false, dirty: false, error: null, conflict: false, value: {} });
}
export function initializeDesktopState(owner: string): Promise<void> {
  const state = useDesktopPersistence.getState();
  if (state.owner === owner && state.loaded) return Promise.resolve();
  if (state.owner === owner && initialization) return initialization;
  if (state.owner !== owner) {
    // Edits entered before the first load belong to that first account, not a later account.
    if (state.owner !== null) resetDesktopState();
    useDesktopPersistence.setState({ owner });
  }
  const token = lifecycle;
  useDesktopPersistence.setState({ loading: true, error: null });
  const operation = (async () => {
    try {
      const snapshot = await invoke<DesktopSnapshot>('read_development_desktop_state');
      if (token !== lifecycle) return;
      if (!valid(snapshot)) throw new Error('desktop_state_invalid');
      revision = snapshot.revision;
      const local = useDesktopPersistence.getState().value;
      const value = { ...snapshot.value };
      for (const key of edited) value[key] = local[key];
      useDesktopPersistence.setState({ value, loaded: true, loading: false, error: null, conflict: false });
      // Initial hydration merges unseen entries in each store; later reloads must
      // honor whole edited slots, including explicit removals from those stores.
      restore(hydrated ? value : snapshot.value);
      hydrated = true;
      if (useDesktopPersistence.getState().dirty) schedule();
    } catch (error) {
      if (token === lifecycle) useDesktopPersistence.setState({ loaded: false, loading: false, error: failure(error) });
    }
  })();
  initialization = operation;
  void operation.finally(() => { if (initialization === operation) initialization = null; });
  return operation;
}
export function desktopSlot<T>(key: string, fallback: T): T {
  const value = restoringValue ?? useDesktopPersistence.getState().value;
  return Object.prototype.hasOwnProperty.call(value, key) ? value[key] as T : fallback;
}
export function setDesktopSlot(key: string, value: unknown): void {
  if (Object.is(useDesktopPersistence.getState().value[key], value)) return;
  if (applying && JSON.stringify(useDesktopPersistence.getState().value[key]) === JSON.stringify(value)) return;
  ++changes; edited.add(key);
  useDesktopPersistence.setState(state => ({ value: { ...state.value, [key]: value }, dirty: true }));
  if (!applying) schedule();
}
export function useDesktopSlot<T>(key: string, fallback: T): [T, (value: T) => void] {
  const value = useDesktopPersistence(state => Object.prototype.hasOwnProperty.call(state.value, key) ? state.value[key] as T : fallback);
  return [value, next => setDesktopSlot(key, next)];
}
function schedule(): void {
  clearTimeout(timer);
  const state = useDesktopPersistence.getState();
  if (!state.loaded || state.error || state.conflict || flight) return;
  timer = window.setTimeout(() => { timer = undefined; void flushDesktopState(); }, 350);
}
/** Single-flight drain: edits made during a save are included before this promise succeeds. */
export async function flushDesktopState(): Promise<boolean> {
  clearTimeout(timer); timer = undefined;
  const expectedLifecycle = lifecycle;
  if (initialization) await initialization;
  if (expectedLifecycle !== lifecycle) return false;
  const state = useDesktopPersistence.getState();
  if (!state.dirty) return !state.error;
  if (!state.loaded || state.conflict || state.error === 'desktop_state_invalid') return false;
  if (flight) return flight;
  const token = lifecycle;
  const operation = (async () => {
    useDesktopPersistence.setState({ saving: true });
    try {
      while (token === lifecycle && useDesktopPersistence.getState().dirty) {
        const captured = changes;
        const value = useDesktopPersistence.getState().value;
        if (new TextEncoder().encode(JSON.stringify(value)).byteLength > MAX_STATE_BYTES) throw new Error('desktop_state_limit');
        const snapshot = await invoke<DesktopSnapshot>('save_development_desktop_state', { input: { expected_revision: revision, value } });
        if (token !== lifecycle) return false;
        if (!valid(snapshot)) throw new Error('desktop_state_invalid');
        revision = snapshot.revision;
        if (captured === changes) { edited.clear(); useDesktopPersistence.setState({ dirty: false, error: null }); }
      }
      if (token !== lifecycle) return false;
      useDesktopPersistence.setState({ saving: false, error: null });
      return true;
    } catch (error) {
      if (token === lifecycle) { const code = failure(error); useDesktopPersistence.setState({ saving: false, error: code, conflict: code === 'desktop_state_conflict' }); }
      return false;
    }
  })();
  flight = operation;
  void operation.finally(() => { if (flight === operation) flight = null; });
  return operation;
}
/** Ordinary save failures retry the same revision. A conflict requires explicit reload/merge. */
export async function retryDesktopState(): Promise<boolean> {
  const state = useDesktopPersistence.getState();
  if (state.conflict) return false;
  if (!state.loaded && state.owner) await initializeDesktopState(state.owner);
  else if (state.loaded) useDesktopPersistence.setState({ error: null });
  return flushDesktopState();
}
/** Explicit conflict recovery retains locally edited slots and re-reads the host revision. */
export async function reloadDesktopState(): Promise<void> {
  const state = useDesktopPersistence.getState();
  if (!state.owner) return;
  useDesktopPersistence.setState({ loaded: false, conflict: false });
  await initializeDesktopState(state.owner);
}
/** Rebase only the account epoch. Native generations/request identities are never rewritten. */
export function rebaseDesktopKey(key: string, epoch: number): string | null {
  try { const parts: unknown = JSON.parse(key); if (!Array.isArray(parts) || !Number.isInteger(parts[0])) return null; parts[0] = epoch; return JSON.stringify(parts); }
  catch { return null; }
}

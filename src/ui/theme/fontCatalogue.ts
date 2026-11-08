import { useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface LocalFont { family: string; monospace: boolean }
export type FontCatalogue =
  | { status: 'idle' | 'loading' | 'failed'; families: readonly LocalFont[] }
  | { status: 'ready'; families: readonly LocalFont[] };
let state: FontCatalogue = { status: 'idle', families: [] };
let pending: Promise<void> | null = null;
let generation = 0;
const listeners = new Set<() => void>();
export const getFontCatalogue = () => state;
export function subscribeFontCatalogue(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
function publish(next: FontCatalogue) {
  state = next;
  for (const listener of listeners) listener();
}
export function validFontFamily(value: unknown): value is string {
  return typeof value === 'string' && value.trim() === value && [...value].length > 0 && [...value].length <= 200 && !/[\u0000-\u001f\u007f-\u009f]/.test(value);
}
/** One shared host request; only an explicit retry rescans after failure. */
export function loadFontCatalogue(retry = false): Promise<void> {
  if (pending) return pending;
  if (state.status === 'ready' || (state.status === 'failed' && !retry)) return Promise.resolve();
  const current = generation;
  publish({ status: 'loading', families: [] });
  pending = (async () => {
    try {
      const response = await invoke<{ success: boolean; data?: unknown }>('list_local_fonts');
      if (!response?.success || !Array.isArray(response.data) || response.data.length === 0) throw new Error('Font catalogue unavailable');
      const names = new Set<string>();
      const families: LocalFont[] = response.data.map((item: unknown) => {
        if (!item || typeof item !== 'object' || !('family' in item) || !validFontFamily(item.family) || !('monospace' in item) || typeof item.monospace !== 'boolean' || names.has(item.family)) throw new Error('Invalid font catalogue');
        names.add(item.family);
        return { family: item.family, monospace: item.monospace };
      });
      families.sort((a, b) => a.family.localeCompare(b.family));
      if (current === generation) publish({ status: 'ready', families });
    } catch {
      if (current === generation) publish({ status: 'failed', families: [] });
    } finally {
      if (current === generation) pending = null;
    }
  })();
  return pending;
}
export function useFontCatalogue(): FontCatalogue {
  return useSyncExternalStore(subscribeFontCatalogue, getFontCatalogue, getFontCatalogue);
}
export function resetFontCatalogueForTesting(): void {
  generation++;
  pending = null;
  publish({ status: 'idle', families: [] });
}

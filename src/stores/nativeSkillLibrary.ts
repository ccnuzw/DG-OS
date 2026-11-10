import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { nativeError, nativeSelectionIdentity } from './nativeExtensions';
import { useDevelopment } from './developmentStore';
import type { NativeResult, NativeSelection } from '../types/nativeExtensions';
import type { NativeDependencyReport, NativeLibrarySearch, NativePickedSkillSource, NativePreparedSkillUpdate, NativeSkillFile, NativeSkillInspection, NativeSkillRequest, NativeSkillSource, NativeVersionPlan, NativeVersionResult } from '../types/nativeSkillLibrary';

export function nativeLibraryKey(selection: NativeSelection): string {
  return JSON.stringify([useDevelopment.getState().epoch, selection.target, selection.scope, selection.project_id, selection.task_id ?? null, nativeSelectionIdentity(selection)]);
}
interface SourceState { value: NativeSkillSource[]; loading: boolean; error: string | null; loaded: boolean }
export interface NativeSourceDraft { input: string; sourceId: string | null; revision: number | null }
export interface NativeVersionRecord { skill: NativeSkillRequest; plan: NativeVersionPlan; result: NativeVersionResult; busy: boolean; error: string | null }
interface LibraryStore {
  sources: Record<string, SourceState>;
  drafts: Record<string, NativeSourceDraft>;
  versions: Record<string, NativeVersionRecord>;
  setDraft(selection: NativeSelection, draft: NativeSourceDraft | null): void;
  loadSources(selection: NativeSelection): Promise<NativeResult<NativeSkillSource[]>>;
  saveSource(selection: NativeSelection, draft: NativeSourceDraft): Promise<NativeResult<NativeSkillSource>>;
  refreshSource(selection: NativeSelection, source: NativeSkillSource): Promise<NativeResult<NativeSkillSource>>;
  removeSource(selection: NativeSelection, source: NativeSkillSource): Promise<NativeResult<void>>;
  search(selection: NativeSelection, query: string, sourceId: string | null): Promise<NativeResult<NativeLibrarySearch>>;
  pick(selection: NativeSelection, kind: 'zip' | 'directory'): Promise<NativeResult<NativePickedSkillSource | null>>;
  release(selection: NativeSelection, handle: string): Promise<NativeResult<void>>;
  inspect(input: NativeSkillRequest): Promise<NativeResult<NativeSkillInspection>>;
  readFile(skill: NativeSkillRequest, path: string): Promise<NativeResult<NativeSkillFile>>;
  dependencies(input: NativeSkillRequest): Promise<NativeResult<NativeDependencyReport>>;
  previewVersion(skill: NativeSkillRequest, program: string): Promise<NativeResult<NativeVersionPlan>>;
  runVersion(skill: NativeSkillRequest, plan: NativeVersionPlan): Promise<NativeResult<NativeVersionResult>>;
  inspectVersion(planId: string): Promise<NativeResult<NativeVersionResult>>;
  prepareUpdate(input: NativeSkillRequest): Promise<NativeResult<NativePreparedSkillUpdate>>;
  reset(): void;
}
const sequences = new Map<string, number>();
export const useNativeSkillLibrary = create<LibraryStore>((set, get) => {
  async function call<T>(command: string, selection: NativeSelection, input: object, dispose?: (value: T) => void): Promise<NativeResult<T>> {
    const context = nativeLibraryKey(selection), epoch = useDevelopment.getState().epoch;
    try {
      const value = await invoke<T>(command, { input });
      if (context === nativeLibraryKey(selection)) return { ok: true, value };
      if (useDevelopment.getState().epoch === epoch) dispose?.(value);
      return { ok: false, code: 'native_extension_StaleGeneration' };
    } catch (error) {
      return { ok: false, code: context === nativeLibraryKey(selection) ? nativeError(error) : 'native_extension_StaleGeneration' };
    }
  }
  function upsert(selection: NativeSelection, source: NativeSkillSource) {
    const key = nativeLibraryKey(selection), previous = get().sources[key];
    sequences.set(key, (sequences.get(key) ?? 0) + 1);
    const value = [...(previous?.value ?? []).filter(item => item.id !== source.id), source].sort((left, right) => left.repository.normalized_source.localeCompare(right.repository.normalized_source));
    set(state => ({ sources: { ...state.sources, [key]: { value, loading: false, error: null, loaded: true } } }));
  }
  async function versionOperation(record: NativeVersionRecord, action: 'run' | 'inspect'): Promise<NativeResult<NativeVersionResult>> {
    const epoch = useDevelopment.getState().epoch, id = record.plan.plan_id;
    set(state => ({ versions: { ...state.versions, [id]: { ...record, busy: true, error: null } } }));
    let result: NativeResult<NativeVersionResult>;
    try {
      const value = await invoke<NativeVersionResult>(`${action}_native_skill_version`, { input: { selection: record.skill.selection, plan_id: id, ...(action === 'run' ? { approved: true } : {}) } });
      result = { ok: true, value };
    } catch (error) { result = { ok: false, code: nativeError(error) }; }
    if (useDevelopment.getState().epoch !== epoch) return { ok: false, code: 'native_extension_StaleGeneration' };
    set(state => ({ versions: { ...state.versions, [id]: { ...record, busy: false, result: result.ok ? result.value : record.result, error: result.ok ? null : result.code } } }));
    return result;
  }
  return {
    sources: {}, drafts: {}, versions: {},
    setDraft(selection, draft) {
      const key = nativeLibraryKey(selection), drafts = { ...get().drafts };
      if (draft) drafts[key] = draft; else delete drafts[key];
      set({ drafts });
    },
    async loadSources(selection) {
      const key = nativeLibraryKey(selection), sequence = (sequences.get(key) ?? 0) + 1;
      sequences.set(key, sequence);
      set(state => ({ sources: { ...state.sources, [key]: { value: state.sources[key]?.value ?? [], loading: true, loaded: state.sources[key]?.loaded ?? false, error: null } } }));
      const result = await call<NativeSkillSource[]>('list_native_skill_sources', selection, selection);
      if (key === nativeLibraryKey(selection) && sequences.get(key) === sequence) {
        set(state => ({ sources: { ...state.sources, [key]: { value: result.ok ? result.value : state.sources[key]?.value ?? [], loading: false, loaded: result.ok || (state.sources[key]?.loaded ?? false), error: result.ok ? null : result.code } } }));
      }
      return result;
    },
    async saveSource(selection, draft) {
      const result = await call<NativeSkillSource>('save_native_skill_source', selection, { selection, id: draft.sourceId, expected_revision: draft.revision, input: draft.input });
      if (result.ok) { upsert(selection, result.value); if (get().drafts[nativeLibraryKey(selection)] === draft) get().setDraft(selection, null); }
      return result;
    },
    async refreshSource(selection, source) {
      const result = await call<NativeSkillSource>('refresh_native_skill_source', selection, { selection, source_id: source.id, expected_revision: source.revision });
      if (result.ok) upsert(selection, result.value);
      return result;
    },
    async removeSource(selection, source) {
      const result = await call<void>('remove_native_skill_source', selection, { selection, source_id: source.id, expected_revision: source.revision });
      if (result.ok) {
        const key = nativeLibraryKey(selection);
        sequences.set(key, (sequences.get(key) ?? 0) + 1);
        set(state => ({ sources: { ...state.sources, [key]: { value: (state.sources[key]?.value ?? []).filter(item => item.id !== source.id), loading: false, loaded: true, error: null } } }));
      }
      return result;
    },
    search: (selection, query, sourceId) => call<NativeLibrarySearch>('search_native_skill_library', selection, { selection, query, source_id: sourceId }),
    pick: (selection, kind) => call<NativePickedSkillSource | null>('pick_native_skill_source', selection, { selection, kind }, value => { if (value) void get().release(selection, value.handle); }),
    release: (selection, handle) => call<void>('release_native_skill_source', selection, { selection, handle }),
    inspect: input => call<NativeSkillInspection>('inspect_native_skill', input.selection, input),
    readFile: (skill, path) => call<NativeSkillFile>('read_native_skill_file', skill.selection, { skill, path }),
    dependencies: input => call<NativeDependencyReport>('inspect_native_skill_dependencies', input.selection, input),
    previewVersion: (skill, program) => call<NativeVersionPlan>('preview_native_skill_version', skill.selection, { skill, program }),
    runVersion(skill, plan) {
      if (get().versions[plan.plan_id] || Object.values(get().versions).some(record => record.busy || !record.result.cleanup_confirmed)) return Promise.resolve({ ok: false, code: 'native_extension_Busy' });
      return versionOperation({ skill, plan, result: { status: 'unknown', output: null, exit_code: null, cleanup_confirmed: false, requirement_satisfied: null }, busy: false, error: null }, 'run');
    },
    inspectVersion(planId) {
      const record = get().versions[planId];
      if (!record || record.busy) return Promise.resolve({ ok: false, code: 'native_extension_Busy' });
      return versionOperation(record, 'inspect');
    },
    prepareUpdate: input => call<NativePreparedSkillUpdate>('prepare_native_skill_update', input.selection, input, value => { void get().release(input.selection, value.handle); }),
    reset() { sequences.clear(); set({ sources: {}, drafts: {}, versions: {} }); },
  };
});
useDevelopment.subscribe((state, previous) => { if (state.epoch !== previous.epoch) useNativeSkillLibrary.getState().reset(); });

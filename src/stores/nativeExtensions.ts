import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopment } from './developmentStore';
import type { DiscoverNativeSkills, GetNativeSkill, NativeDraft, NativeMcp, NativeOperation, NativePending, NativePreview, NativeReceipt, NativeResult, NativeSelection, NativeSkill, NativeSkillPreview, NativeSkillRules, NativeSkills, NativeVisibilityPreview, PreviewNativeMcp, PreviewNativeSkill, PreviewNativeVisibility } from '../types/nativeExtensions';

const codes = new Set(['AccessDenied', 'UnsafePath', 'RootChanged', 'InvalidFormat', 'Limit', 'Unsupported', 'Conflict', 'Busy', 'Expired', 'NotFound', 'Unavailable', 'RecoveryRequired', 'StaleGeneration'].map(code => `native_extension_${code}`));
for (const code of ['session_invalid', 'session_closed', 'session_busy', 'session_exists', 'bridge_unavailable', 'request_capacity', 'provider_revision_changed', 'provider_binding_unconfirmed', 'cli_incompatible', 'provider_required', 'provider_unavailable', 'provider_incompatible', 'credential_unavailable', 'legacy_session_read_only', 'capability_revoked', 'stale_generation', 'invalid_request_id', 'request_conflict', 'project_not_found', 'project_archived', 'workspace_changed', 'storage_failed', 'cancelled', 'maintenance_active']) codes.add(code);
export function nativeError(error: unknown): string { return typeof error === 'string' && codes.has(error) ? error : 'native_extension_Unavailable'; }
const key = (selection: NativeSelection) => JSON.stringify([useDevelopment.getState().epoch, selection.target, selection.scope, selection.project_id, selection.task_id ?? null]);
// Keep the public selection key stable, but never reuse observations after a
// known root relocation/replacement or task-workspace evidence change.
export function nativeSelectionIdentity(selection: NativeSelection): string {
  const state = useDevelopment.getState();
  if (selection.scope === 'OsUser') return JSON.stringify([state.workspaceOwner, 'os-user']);
  const project = state.projects.find(project => project.id === selection.project_id);
  const task = state.tasks.find(task => task.id === selection.task_id);
  const workspaces = selection.task_id ? [...new Set(state.sessions.filter(session => session.project_id === selection.project_id && session.task_id === selection.task_id && session.engine === selection.target.toLowerCase()).map(session => session.workspace))].sort() : [];
  return JSON.stringify([state.workspaceOwner, project ? [project.owner, project.path, project.directory_identity, project.archived] : null, task ? [task.owner, task.project_id] : null, workspaces]);
}
interface ReadState<T> { value: T | null; loading: boolean; error: string | null; stale: boolean }
interface NativeStore {
  epoch: number;
  skills: Record<string, ReadState<NativeSkills>>;
  mcp: Record<string, ReadState<NativeMcp>>;
  rules: Record<string, ReadState<NativeSkillRules>>;
  receipts: Record<string, NativeReceipt>;
  drafts: Record<string, NativeDraft>;
  pending: Record<string, NativePending>;
  busy: Record<string, true>;
  setDraft(id: string, draft: NativeDraft): void;
  discardDrafts(expected: Record<string, NativeDraft>): boolean;
  setPending(id: string, pending: NativePending): void;
  begin(id: string, preview: boolean): boolean;
  end(id: string): void;
  clearCommittedDraft(pending: NativePending): void;
  discoverSkills(input: DiscoverNativeSkills): Promise<NativeResult<NativeSkills>>;
  discoverSkillRules(input: NativeSelection): Promise<NativeResult<NativeSkillRules>>;
  getSkill(input: GetNativeSkill): Promise<NativeResult<NativeSkill>>;
  discoverMcp(input: NativeSelection): Promise<NativeResult<NativeMcp>>;
  previewSkill(input: PreviewNativeSkill): Promise<NativeResult<NativeSkillPreview>>;
  previewVisibility(input: PreviewNativeVisibility): Promise<NativeResult<NativeVisibilityPreview>>;
  previewMcp(input: PreviewNativeMcp): Promise<NativeResult<NativePreview>>;
  commit(input: NativeOperation): Promise<NativeResult<NativeReceipt>>;
  cancel(input: NativeOperation): Promise<NativeResult<NativeReceipt>>;
  inspect(input: NativeOperation): Promise<NativeResult<NativeReceipt>>;
  reset(): void;
}
const reads = new Map<string, number>();
const readFlights = new Map<string, Promise<NativeResult<unknown>>>();
const operations = new Set<string>();
const readContexts = new Map<string, { selection: NativeSelection; identity: string; kind: 'skills' | 'mcp' | 'rules'; id: string }>();
export const useNativeExtensions = create<NativeStore>((set, get) => {
  async function call<T>(command: string, input: object): Promise<NativeResult<T>> {
    const epoch = get().epoch;
    const account = useDevelopment.getState().epoch;
    const selection = 'target' in input && 'scope' in input && 'project_id' in input ? input as NativeSelection : null;
    const identity = selection ? nativeSelectionIdentity(selection) : null;
    const invalidated = () => epoch !== get().epoch || account !== useDevelopment.getState().epoch ? 'session_invalid' : selection && nativeSelectionIdentity(selection) !== identity ? 'native_extension_RootChanged' : null;
    try {
      const value = await invoke<T>(command, { input });
      const code = invalidated();
      return code ? { ok: false, code } : { ok: true, value };
    } catch (error) { return { ok: false, code: invalidated() ?? nativeError(error) }; }
  }
  async function read<T>(kind: 'skills' | 'mcp' | 'rules', command: string, input: NativeSelection & { query?: string }): Promise<NativeResult<T>> {
    const id = key(input) + (kind === 'skills' ? `:${input.query ?? ''}` : '');
    const request = `${kind}:${id}`;
    const identity = nativeSelectionIdentity(input);
    if (readContexts.get(request)?.identity !== identity) {
      reads.set(request, (reads.get(request) ?? 0) + 1);
      readFlights.delete(request);
      const entries = { ...get()[kind] }; delete entries[id];
      set({ [kind]: entries });
    }
    readContexts.set(request, { selection: { ...input }, identity, kind, id });
    const existing = readFlights.get(request);
    if (existing) return existing as Promise<NativeResult<T>>;
    const sequence = (reads.get(request) ?? 0) + 1;
    reads.set(request, sequence);
    const epoch = get().epoch;
    const previous = get()[kind][id];
    set({ [kind]: { ...get()[kind], [id]: { value: previous?.value ?? null, loading: true, stale: true, error: null } } });
    const flight = call<T>(command, input).then<NativeResult<T>>(result => {
      if (epoch !== get().epoch) return { ok: false, code: 'session_invalid' };
      if (reads.get(request) !== sequence) return result.ok ? { ok: false, code: 'native_extension_Conflict' } : result;
      set({ [kind]: { ...get()[kind], [id]: result.ok ? { value: result.value, loading: false, stale: false, error: null } : { value: previous?.value ?? null, loading: false, stale: true, error: result.code } } });
      return result;
    }).finally(() => { if (readFlights.get(request) === flight) readFlights.delete(request); });
    readFlights.set(request, flight as Promise<NativeResult<unknown>>);
    return flight;
  }
  async function operation(command: string, input: NativeOperation): Promise<NativeResult<NativeReceipt>> {
    if (operations.has(input.operation_id)) return { ok: false, code: 'native_extension_Busy' };
    operations.add(input.operation_id);
    try {
      const result = await call<NativeReceipt>(command, input);
      if (result.ok) set({ receipts: { ...get().receipts, [input.operation_id]: result.value } });
      if (result.ok && result.value.state === 'Committed') {
        const pending = Object.values(get().pending).find(item => item.receipt.operation_id === input.operation_id);
        if (pending) {
          const id = key(pending.selection);
          const kinds = pending.kind === 'skills' ? ['skills', 'rules'] as const : ['mcp'] as const;
          for (const kind of kinds) {
            for (const request of reads.keys()) {
              if (request === `${kind}:${id}` || request.startsWith(`${kind}:${id}:`)) {
                reads.set(request, (reads.get(request) ?? 0) + 1);
                readFlights.delete(request);
              }
            }
            const entries = Object.fromEntries(Object.entries(get()[kind]).map(([entry, state]) => [entry, entry === id || entry.startsWith(`${id}:`) ? { ...state, loading: false, stale: true } : state]));
            set({ [kind]: entries });
          }
        }
      }
      return result;
    } finally { operations.delete(input.operation_id); }
  }
  return {
    epoch: 0, skills: {}, mcp: {}, rules: {}, receipts: {}, drafts: {}, pending: {}, busy: {},
    setDraft(id, draft) { set({ drafts: { ...get().drafts, [id]: draft } }); },
    discardDrafts(expected) {
      if (Object.entries(expected).some(([id, draft]) => get().drafts[id] !== draft)) return false;
      const drafts = { ...get().drafts };
      for (const id of Object.keys(expected)) delete drafts[id];
      set({ drafts }); return true;
    },
    setPending(id, pending) { set({ pending: { ...get().pending, [id]: pending } }); },
    begin(id, preview) {
      const pending = get().pending[id];
      if (get().busy[id] || (preview && pending && pending.mode !== 'result')) return false;
      set({ busy: { ...get().busy, [id]: true } }); return true;
    },
    end(id) { const busy = { ...get().busy }; delete busy[id]; set({ busy }); },
    clearCommittedDraft(pending) {
      // A draft edited after preview is a new edit and must survive that commit.
      if (get().drafts[pending.draft_key] !== pending.draft) return;
      const drafts = { ...get().drafts }; delete drafts[pending.draft_key]; set({ drafts });
    },
    discoverSkills: input => read<NativeSkills>('skills', 'discover_native_skills', input),
    discoverSkillRules: input => read<NativeSkillRules>('rules', 'discover_native_skill_rules', input),
    getSkill: input => call<NativeSkill>('get_native_skill', input),
    discoverMcp: input => read<NativeMcp>('mcp', 'discover_native_mcp', input),
    previewSkill: input => call<NativeSkillPreview>('preview_native_skill', input),
    previewVisibility: input => call<NativeVisibilityPreview>('preview_native_skill_visibility', input),
    previewMcp: input => call<NativePreview>('preview_native_mcp', input),
    commit: input => operation('commit_native_operation', input),
    cancel: input => operation('cancel_native_operation', input),
    inspect: input => operation('inspect_native_operation', input),
    reset() { reads.clear(); readFlights.clear(); readContexts.clear(); operations.clear(); set({ epoch: get().epoch + 1, skills: {}, mcp: {}, rules: {}, receipts: {}, drafts: {}, pending: {}, busy: {} }); },
  };
});
// Existing account reload resets Development before navigating; invalidate synchronously.
useDevelopment.subscribe((state, previous) => {
  if (state.epoch !== previous.epoch) { useNativeExtensions.getState().reset(); return; }
  if (state.workspaceOwner === previous.workspaceOwner && state.projects === previous.projects && state.tasks === previous.tasks && state.sessions === previous.sessions) return;
  const changes: Partial<Pick<NativeStore, 'skills' | 'mcp' | 'rules'>> = {};
  for (const [request, context] of readContexts) {
    if (nativeSelectionIdentity(context.selection) === context.identity) continue;
    reads.set(request, (reads.get(request) ?? 0) + 1);
    readFlights.delete(request); readContexts.delete(request);
    const entries = { ...(changes[context.kind] ?? useNativeExtensions.getState()[context.kind]) };
    delete entries[context.id];
    Object.assign(changes, { [context.kind]: entries });
  }
  if (Object.keys(changes).length) useNativeExtensions.setState(changes);
});

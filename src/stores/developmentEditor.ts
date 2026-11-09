import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopment } from './developmentStore';
import { readDevelopmentFile, type DevFileView } from './developmentReview';
import { hostCode } from './hostError';
import { confirmDiscard } from '../workspace/closeCoordinator';
import { t } from '../i18n';
import { useDevelopmentDrafts } from './developmentDrafts';

interface EditorState {
  projectId: string | null; taskId: string | null; path: string | null;
  file: DevFileView | null; draft: string; disk: DevFileView | null;
  busy: boolean; saving: boolean; error: string | null;
  open(projectId: string, taskId: string | null, path: string): Promise<void>;
  edit(text: string): void; save(): Promise<void>; reloadDisk(): Promise<void>; reset(): void;
}
let revision = 0;
export const useDevelopmentEditor = create<EditorState>((set, get) => ({
  projectId: null, taskId: null, path: null, file: null, draft: '', disk: null, busy: false, saving: false, error: null,
  async open(projectId, taskId, path) {
    if (developmentEditorMutationAffects(projectId, taskId, path)) return;
    const ticket = ++revision;
    const epoch = useDevelopment.getState().epoch;
    set({ projectId, taskId, path, file: null, draft: '', disk: null, busy: true, saving: false, error: null });
    const result = await readDevelopmentFile({ project_id: projectId, task_id: taskId, path });
    if (ticket !== revision || epoch !== useDevelopment.getState().epoch) return;
    set(result.ok ? { file: result.value, draft: result.value.text ?? '', busy: false } : { error: result.code, busy: false });
  },
  edit(draft) { set({ draft }); },
  async save() {
    const { projectId, taskId, path, file, draft, saving } = get();
    if (!projectId || !taskId || !path || !file?.hash || file.text === null || saving || get().error === 'file_changed' || developmentEditorMutationAffects(projectId, taskId, path)) return;
    const ticket = revision;
    const epoch = useDevelopment.getState().epoch;
    set({ saving: true, error: null });
    try {
      const saved = await invoke<DevFileView>('save_development_file', { input: { project_id: projectId, task_id: taskId, path, expected_hash: file.hash, text: draft } });
      if (ticket !== revision || epoch !== useDevelopment.getState().epoch) return;
      // Edits typed while saving remain a draft against the newly saved version.
      set({ file: saved, disk: null, saving: false });
    } catch (error) {
      if (ticket !== revision || epoch !== useDevelopment.getState().epoch) return;
      const code = hostCode(error);
      set({ error: code, saving: false });
      if (code === 'file_changed') {
        const disk = await readDevelopmentFile({ project_id: projectId, task_id: taskId, path });
        if (ticket !== revision || epoch !== useDevelopment.getState().epoch) return;
        if (disk.ok) set({ disk: disk.value });
      }
    }
  },
  async reloadDisk() {
    if (!await guardDevelopmentDraft()) return;
    const { projectId, taskId, path } = get();
    if (projectId && path) await get().open(projectId, taskId, path);
  },
  reset() { ++revision; set({ projectId: null, taskId: null, path: null, file: null, draft: '', disk: null, busy: false, saving: false, error: null }); },
}));
export function hasDevelopmentDraft(): boolean {
  const { file, draft } = useDevelopmentEditor.getState();
  return file?.text !== null && file !== null && draft !== file.text;
}
export function developmentEditorMutationAffects(projectId: string, taskId: string | null, path: string): boolean {
  const epoch = useDevelopment.getState().epoch;
  return Object.entries(useDevelopmentDrafts.getState().entries).some(([key, draft]) => {
    let identity: unknown[];
    try { identity = JSON.parse(key); } catch { return false; }
    if (identity[0] !== epoch || identity[1] !== 'delivery' || identity[2] !== projectId || identity[3] !== taskId) return false;
    const value = draft.value;
    if (!value || typeof value !== 'object' || !('operation' in value) || !value.operation || typeof value.operation !== 'object') return false;
    const operation = value.operation;
    if (!('path' in operation) || typeof operation.path !== 'string' || !('status' in operation) || !['pending', 'unknown'].includes(String(operation.status)) || !('action' in operation) || !['rename', 'delete'].includes(String(operation.action))) return false;
    return path === operation.path || path.startsWith(`${operation.path}/`);
  });
}
export async function guardDevelopmentDraft(allowMutation = false): Promise<boolean> {
  const current = useDevelopmentEditor.getState();
  if (!allowMutation && current.projectId && current.path && developmentEditorMutationAffects(current.projectId, current.taskId, current.path)) return false;
  if (!hasDevelopmentDraft()) return true;
  const { draft, file } = useDevelopmentEditor.getState();
  return await confirmDiscard({ title: t('dev.discardDraft'), content: t('dev.discardDraft.file', { path: file?.path ?? '' }), confirmText: t('dev.discardDraft') }) && useDevelopmentEditor.getState().draft === draft && useDevelopmentEditor.getState().file === file;
}
export async function discardDevelopmentDraft(): Promise<boolean> {
  const epoch = useDevelopment.getState().epoch;
  if (!await guardDevelopmentDraft() || epoch !== useDevelopment.getState().epoch) return false;
  const current = useDevelopmentEditor.getState();
  ++revision;
  useDevelopmentEditor.setState({ draft: current.file?.text ?? '', disk: null, error: null, saving: false, busy: false });
  return true;
}
useDevelopment.subscribe((state, previous) => {
  if (state.epoch !== previous.epoch) useDevelopmentEditor.getState().reset();
});

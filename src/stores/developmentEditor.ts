import { createElement } from 'react';
import { Button, Modal, type ModalHandle } from '@dgos/ui';
import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { useDevelopment } from './developmentStore';
import { readDevelopmentFile, type DevFileView } from './developmentReview';
import { hostCode } from './hostError';
import { t } from '../i18n';
import { useDevelopmentDrafts } from './developmentDrafts';
import { desktopSlot, onDesktopStateRestored, rebaseDesktopKey, setDesktopSlot } from './developmentDesktop';
import { contextGeneration } from './developmentSelection';

export interface EditorDocument {
  id: string; projectId: string; taskId: string | null; workspacePath: string; path: string;
  file: DevFileView | null; draft: string; disk: DevFileView | null;
  busy: boolean; saving: boolean; error: string | null; revision: number;
  /** Changes on typing, independently of saves and read revisions. */
  editRevision?: number;
  /** Cached baseline must be revalidated before an explicit disk save. */
  restored?: boolean;
}
export interface DocumentTab { id: string; projectId: string; taskId: string | null; workspacePath: string; path: string; mode: 'file' | 'diff'; source: 'task' | 'index' | 'worktree'; line?: number }
const empty = { projectId: null, taskId: null, path: null, file: null, draft: '', disk: null, busy: false, saving: false, error: null };
interface EditorState {
  projectId: string | null; taskId: string | null; path: string | null;
  file: DevFileView | null; draft: string; disk: DevFileView | null; busy: boolean; saving: boolean; error: string | null;
  activeId: string | null; documents: Record<string, EditorDocument>; tabs: DocumentTab[]; activeTabs: Record<string, string>;
  open(projectId: string, taskId: string | null, path: string, workspacePath?: string, reload?: boolean): Promise<void>;
  edit(text: string, id?: string): void; save(id?: string): Promise<void>; reloadDisk(id?: string): Promise<void>; reset(): void;
}
export const documentScope = (projectId: string, taskId: string | null, workspacePath = '') => JSON.stringify([useDevelopment.getState().epoch, projectId, taskId, workspacePath]);
export const documentId = (projectId: string, taskId: string | null, path: string, workspacePath = '') => JSON.stringify([useDevelopment.getState().epoch, projectId, taskId, workspacePath, path]);
export const documentDirty = (document: EditorDocument) => document.file?.text != null && document.draft !== document.file.text;
const affected = (path: string, source: string) => source === '' || path === source || path.startsWith(`${source}/`);
const matches = (document: Pick<EditorDocument, 'projectId' | 'taskId' | 'path' | 'workspacePath'>, projectId: string, taskId: string | null, paths: string[], workspacePath?: string) => document.projectId === projectId && document.taskId === taskId && (workspacePath === undefined || workspacePath === document.workspacePath) && paths.some(path => affected(document.path, path));
let sequence = 0;
export const useDevelopmentEditor = create<EditorState>((set, get) => {
  function activate(id: string | null, documents = get().documents) {
    const document = id ? documents[id] : null;
    return { activeId: id, ...(document ?? empty) };
  }
  function update(id: string, patch: Partial<EditorDocument>) {
    set(state => {
      if (!state.documents[id]) return state;
      const documents = { ...state.documents, [id]: { ...state.documents[id], ...patch } };
      return { documents, ...(state.activeId === id ? activate(id, documents) : {}) };
    });
  }
  return {
    ...empty, activeId: null, documents: {}, tabs: [], activeTabs: {},
    async open(projectId, taskId, path, workspacePath = '', reload = false) {
      if (developmentEditorMutationAffects(projectId, taskId, path)) return;
      const id = documentId(projectId, taskId, path, workspacePath);
      const previous = get().documents[id];
      if (previous && !reload && !previous.restored) { set(activate(id)); return; }
      if (previous?.busy) { set(activate(id)); return; }
      const revision = ++sequence, epoch = useDevelopment.getState().epoch;
      const document: EditorDocument = previous && !reload
        ? { ...previous, busy: true, revision }
        : { id, projectId, taskId, workspacePath, path, file: null, draft: '', disk: null, busy: true, saving: false, error: null, revision };
      const documents = { ...get().documents, [id]: document };
      set({ documents, ...activate(id, documents) });
      const result = await readDevelopmentFile({ project_id: projectId, task_id: taskId, path });
      if (epoch !== useDevelopment.getState().epoch || get().documents[id]?.revision !== revision) return;
      if (!result.ok) { update(id, { error: result.code, busy: false }); return; }
      const current = get().documents[id];
      if (previous && !reload && documentDirty(current)) {
        if (previous.file?.hash !== result.value.hash) update(id, { disk: result.value, error: 'file_changed', busy: false, restored: false });
        else update(id, { file: result.value, draft: current.draft, disk: null, error: null, busy: false, restored: false });
      } else update(id, { file: result.value, draft: result.value.text ?? '', disk: null, error: null, busy: false, restored: false });
    },
    edit(draft, id = get().activeId ?? '') {
      const document = get().documents[id];
      if (document && document.draft !== draft) update(id, { draft, editRevision: (document.editRevision ?? 0) + 1 });
    },
    async save(id = get().activeId ?? '') {
      const document = get().documents[id];
      if (!document) return;
      const { projectId, taskId, path, file, draft, saving, revision } = document;
      if (!taskId || !file?.hash || file.text === null || saving || document.restored || document.busy || document.error === 'file_changed' || developmentEditorMutationAffects(projectId, taskId, path)) return;
      const epoch = useDevelopment.getState().epoch;
      const valid = () => epoch === useDevelopment.getState().epoch && get().documents[id]?.revision === revision;
      update(id, { saving: true, error: null });
      try {
        const saved = await invoke<DevFileView>('save_development_file', { input: { project_id: projectId, task_id: taskId, path, expected_hash: file.hash, text: draft } });
        if (valid()) update(id, { file: saved, disk: null, saving: false });
      } catch (error) {
        if (!valid()) return;
        const code = hostCode(error);
        update(id, { error: code, saving: false });
        if (code === 'file_changed') {
          const disk = await readDevelopmentFile({ project_id: projectId, task_id: taskId, path });
          if (valid() && disk.ok) update(id, { disk: disk.value });
        }
      }
    },
    async reloadDisk(id = get().activeId ?? '') {
      const document = get().documents[id];
      if (!document || document.busy || document.saving) return;
      // Recovery revalidation is a read, not consent to replace the local draft.
      // Keep the cached baseline on failure and compare it with a successful read.
      if (document.restored) {
        await get().open(document.projectId, document.taskId, document.path, document.workspacePath);
        return;
      }
      const guarded = await guardDocuments([document]);
      if (!guarded?.valid()) return;
      await get().open(document.projectId, document.taskId, document.path, document.workspacePath, true);
    },
    reset() { ++sequence; set({ ...empty, activeId: null, documents: {}, tabs: [], activeTabs: {} }); },
  };
});
export function hasDevelopmentDraft(): boolean { return Object.values(useDevelopmentEditor.getState().documents).some(documentDirty); }
export function developmentEditorMutationAffects(projectId: string, taskId: string | null, path: string): boolean {
  const epoch = useDevelopment.getState().epoch;
  return Object.entries(useDevelopmentDrafts.getState().entries).some(([key, draft]) => {
    let identity: unknown[];
    try { identity = JSON.parse(key); } catch { return false; }
    if (identity[0] !== epoch || identity[1] !== 'delivery' || identity[2] !== projectId || identity[3] !== taskId) return false;
    const value = draft.value;
    if (!value || typeof value !== 'object' || !('operation' in value) || !value.operation || typeof value.operation !== 'object') return false;
    const operation = value.operation;
    if (!('status' in operation) || !['pending', 'unknown'].includes(String(operation.status)) || !('action' in operation) || !['rename', 'delete', 'restore_hunk', 'restore', 'pull', 'switch', 'merge'].includes(String(operation.action))) return false;
    const paths = 'paths' in operation && Array.isArray(operation.paths) ? operation.paths.filter((entry): entry is string => typeof entry === 'string') : 'path' in operation && typeof operation.path === 'string' ? [operation.path] : [];
    return paths.some(source => affected(path, source) || affected(source, path));
  });
}
interface GuardedDocuments { documents: EditorDocument[]; valid(): boolean }
type DraftDecision = 'save' | 'discard' | 'cancel';
const canSaveDocument = (document: EditorDocument) => Boolean(document.taskId && document.file?.hash && document.file.text !== null && !document.restored && !document.busy && !document.saving && document.error !== 'file_changed' && !developmentEditorMutationAffects(document.projectId, document.taskId, document.path));
function decideDrafts(documents: EditorDocument[]): Promise<DraftDecision> {
  const canSave = documents.every(canSaveDocument);
  let resolve!: (decision: DraftDecision) => void;
  const promise = new Promise<DraftDecision>(done => { resolve = done; });
  let settled = false;
  let handle: ModalHandle | undefined;
  const finish = (decision: DraftDecision) => {
    if (settled) return;
    settled = true;
    handle?.close();
    resolve(decision);
  };
  handle = Modal.confirm({
    title: t('workbenchFiles.unsavedTitle'), icon: 'warning',
    content: createElement('div', { className: 'flex flex-col gap-2' },
      createElement('p', null, t('workbenchFiles.unsavedDetail')),
      createElement('ul', null, documents.map(document => createElement('li', { key: document.id, className: 'break-all' }, document.path))),
      !canSave ? createElement('p', { role: 'status' }, t('workbenchFiles.unsavedSaveUnavailable')) : null),
    footer: createElement('div', { className: 'flex flex-wrap justify-end gap-2' },
      createElement(Button, { variant: 'secondary', onClick: () => finish('cancel'), children: t('workbenchFiles.returnToEdit') }),
      createElement(Button, { variant: 'danger', onClick: () => finish('discard'), children: t('workbenchFiles.discardAndContinue') }),
      createElement(Button, { disabled: !canSave, onClick: () => finish('save'), children: t('workbenchFiles.saveAndContinue') })),
    onCancel: () => finish('cancel'),
  });
  if (settled) handle.close();
  return promise;
}
async function guardDocuments(documents: EditorDocument[], allowMutation = false): Promise<GuardedDocuments | null> {
  const { epoch, selection } = useDevelopment.getState(), generation = contextGeneration();
  const contextValid = () => epoch === useDevelopment.getState().epoch && selection === useDevelopment.getState().selection && generation === contextGeneration();
  const blocked = (document: EditorDocument) => document.saving || document.busy || !allowMutation && developmentEditorMutationAffects(document.projectId, document.taskId, document.path);
  let approved = documents;
  const valid = () => contextValid() && approved.every(document => useDevelopmentEditor.getState().documents[document.id] === document && !blocked(document));
  if (!valid()) return null;
  const dirty = documents.filter(documentDirty);
  if (!dirty.length) return { documents: approved, valid };
  const decision = await decideDrafts(dirty);
  if (decision === 'cancel' || !valid()) return null;
  if (decision === 'save') {
    if (!dirty.every(canSaveDocument)) return null;
    await Promise.all(dirty.map(document => {
      if (!contextValid() || useDevelopmentEditor.getState().documents[document.id] !== document) return;
      return useDevelopmentEditor.getState().save(document.id);
    }));
    if (!contextValid()) return null;
    const saved = new Set(dirty.map(document => document.id));
    const current = documents.map(document => useDevelopmentEditor.getState().documents[document.id]);
    if (current.some((document, index) => {
      const original = documents[index];
      if (!document || blocked(document)) return true;
      if (!saved.has(original.id)) return document !== original;
      return document.revision !== original.revision || document.editRevision !== original.editRevision || document.draft !== original.draft || document.file === original.file || document.file?.text !== original.draft || !document.file?.hash || document.error !== null || document.disk !== null || document.restored || documentDirty(document);
    })) return null;
    approved = current;
  }
  return { documents: approved, valid };
}
export async function guardDevelopmentDocuments(projectId: string, taskId: string | null, paths: string[], workspacePath?: string, allowMutation = false): Promise<boolean> {
  if (!allowMutation && paths.some(path => developmentEditorMutationAffects(projectId, taskId, path))) return false;
  const documents = Object.values(useDevelopmentEditor.getState().documents).filter(document => matches(document, projectId, taskId, paths, workspacePath));
  const guarded = await guardDocuments(documents, allowMutation);
  return Boolean(guarded?.valid() && (allowMutation || !paths.some(path => developmentEditorMutationAffects(projectId, taskId, path))) && Object.values(useDevelopmentEditor.getState().documents).filter(document => matches(document, projectId, taskId, paths, workspacePath)).length === guarded.documents.length);
}
export async function guardDevelopmentDraft(allowMutation = false): Promise<boolean> {
  const state = useDevelopmentEditor.getState();
  if (!allowMutation && state.tabs.some(tab => developmentEditorMutationAffects(tab.projectId, tab.taskId, tab.path))) return false;
  const guarded = await guardDocuments(Object.values(state.documents), allowMutation);
  return Boolean(guarded?.valid() && (allowMutation || !useDevelopmentEditor.getState().tabs.some(tab => developmentEditorMutationAffects(tab.projectId, tab.taskId, tab.path))) && Object.keys(useDevelopmentEditor.getState().documents).length === guarded.documents.length);
}
function resetDocumentDrafts(documents: EditorDocument[]): void {
  useDevelopmentEditor.setState(state => {
    const next = { ...state.documents };
    for (const document of documents) next[document.id] = { ...document, draft: document.file?.text ?? '', editRevision: (document.editRevision ?? 0) + 1 };
    return { documents: next, ...(state.activeId && next[state.activeId] ? next[state.activeId] : {}) };
  });
}
export async function cancelDevelopmentDocumentEdit(id: string): Promise<boolean> {
  const document = useDevelopmentEditor.getState().documents[id];
  if (!document) return false;
  const guarded = await guardDocuments([document]);
  if (!guarded?.valid()) return false;
  resetDocumentDrafts(guarded.documents);
  return true;
}
export async function discardDevelopmentDraft(): Promise<boolean> {
  const state = useDevelopmentEditor.getState();
  if (state.tabs.some(tab => developmentEditorMutationAffects(tab.projectId, tab.taskId, tab.path))) return false;
  const guarded = await guardDocuments(Object.values(state.documents));
  if (!guarded?.valid() || Object.keys(useDevelopmentEditor.getState().documents).length !== guarded.documents.length || useDevelopmentEditor.getState().tabs.some(tab => developmentEditorMutationAffects(tab.projectId, tab.taskId, tab.path))) return false;
  resetDocumentDrafts(guarded.documents);
  return true;
}

/** Collect project-only consent now; discard unchanged drafts only after unlink succeeds. */
export async function prepareProjectDocumentRemoval(projectId: string): Promise<{ valid(): boolean; discard(): void } | null> {
  const epoch = useDevelopment.getState().epoch;
  const projectDocuments = () => Object.values(useDevelopmentEditor.getState().documents).filter(document => document.projectId === projectId);
  const blocked = () => useDevelopmentEditor.getState().tabs.some(tab => tab.projectId === projectId && developmentEditorMutationAffects(tab.projectId, tab.taskId, tab.path));
  if (blocked()) return null;
  const guarded = await guardDocuments(projectDocuments());
  if (!guarded) return null;
  const valid = () => guarded.valid() && !blocked() && projectDocuments().length === guarded.documents.length;
  if (!valid()) return null;
  return {
    valid,
    discard() {
      if (epoch !== useDevelopment.getState().epoch) return;
      const current = useDevelopmentEditor.getState().documents;
      resetDocumentDrafts(guarded.documents.filter(document => current[document.id] === document));
    },
  };
}
export function openDevelopmentDocument(tab: Omit<DocumentTab, 'id'>): void {
  const id = `${documentId(tab.projectId, tab.taskId, tab.path, tab.workspacePath)}:${tab.mode}`;
  const scope = documentScope(tab.projectId, tab.taskId, tab.workspacePath);
  useDevelopmentEditor.setState(state => ({ tabs: state.tabs.some(item => item.id === id) ? state.tabs.map(item => item.id === id ? { ...tab, id } : item) : [...state.tabs, { ...tab, id }], activeTabs: { ...state.activeTabs, [scope]: id } }));
}
export async function closeDevelopmentDocuments(ids: string[]): Promise<boolean> {
  const state = useDevelopmentEditor.getState(), tabs = state.tabs.filter(tab => ids.includes(tab.id));
  if (tabs.some(tab => developmentEditorMutationAffects(tab.projectId, tab.taskId, tab.path))) return false;
  const documentIds = new Set(tabs.filter(tab => tab.mode === 'file').map(tab => documentId(tab.projectId, tab.taskId, tab.path, tab.workspacePath)));
  const documents = Object.values(state.documents).filter(document => documentIds.has(document.id));
  const guarded = await guardDocuments(documents);
  if (!guarded?.valid() || useDevelopmentEditor.getState().tabs !== state.tabs || tabs.some(tab => developmentEditorMutationAffects(tab.projectId, tab.taskId, tab.path))) return false;
  useDevelopmentEditor.setState(current => {
    const remaining = current.tabs.filter(tab => !ids.includes(tab.id));
    const next = { ...current.documents }; for (const id of documentIds) delete next[id];
    const activeTabs = { ...current.activeTabs };
    for (const [scope, id] of Object.entries(activeTabs)) if (ids.includes(id)) {
      const scoped = remaining.filter(tab => documentScope(tab.projectId, tab.taskId, tab.workspacePath) === scope);
      activeTabs[scope] = scoped[scoped.length - 1]?.id ?? '';
    }
    return { tabs: remaining, documents: next, activeTabs, ...(current.activeId && documentIds.has(current.activeId) ? { ...empty, activeId: null } : {}) };
  });
  return true;
}
export function invalidateDevelopmentDocuments(projectId: string, taskId: string | null, paths: string[], workspacePath?: string): void {
  useDevelopmentEditor.setState(state => {
    const documents = Object.fromEntries(Object.entries(state.documents).filter(([, document]) => !matches(document, projectId, taskId, paths, workspacePath)));
    return { documents, tabs: state.tabs.filter(tab => !matches(tab, projectId, taskId, paths, workspacePath)), ...(state.activeId && !documents[state.activeId] ? { ...empty, activeId: null } : {}) };
  });
}
export function remapDevelopmentDocuments(projectId: string, taskId: string | null, path: string, destination: string, workspacePath: string): void {
  useDevelopmentEditor.setState(state => {
    const remap = <T extends Pick<EditorDocument, 'projectId' | 'taskId' | 'path' | 'workspacePath'>>(item: T): T => matches(item, projectId, taskId, [path], workspacePath) ? { ...item, path: `${destination}${item.path.slice(path.length)}` } : item;
    const documents: Record<string, EditorDocument> = {};
    let activeId = state.activeId;
    for (const document of Object.values(state.documents)) {
      const changed = remap(document), id = documentId(changed.projectId, changed.taskId, changed.path, changed.workspacePath);
      documents[id] = changed === document ? document : { ...changed, id, revision: ++sequence, file: changed.file ? { ...changed.file, path: changed.path } : null };
      if (state.activeId === document.id) activeId = id;
    }
    const idMap = new Map<string, string>();
    const tabs = state.tabs.map(tab => { const changed = remap(tab), id = `${documentId(changed.projectId, changed.taskId, changed.path, changed.workspacePath)}:${changed.mode}`; idMap.set(tab.id, id); return { ...changed, id }; });
    return { documents, tabs, activeId, activeTabs: Object.fromEntries(Object.entries(state.activeTabs).map(([scope, id]) => [scope, idMap.get(id) ?? id])), ...(activeId && documents[activeId] ? documents[activeId] : {}) };
  });
}
useDevelopment.subscribe((state, previous) => { if (state.epoch !== previous.epoch) useDevelopmentEditor.getState().reset(); });

onDesktopStateRestored(() => {
  const saved = desktopSlot<{ documents?: EditorDocument[]; tabs?: DocumentTab[]; activeId?: string | null; activeTabs?: Record<string, string> }>('editor', {});
  const epoch = useDevelopment.getState().epoch;
  const documents: Record<string, EditorDocument> = {};
  for (const value of saved.documents ?? []) {
    if (!value || typeof value.projectId !== 'string' || typeof value.path !== 'string' || typeof value.workspacePath !== 'string' || typeof value.draft !== 'string' || !(value.taskId === null || typeof value.taskId === 'string')) continue;
    const id = documentId(value.projectId, value.taskId, value.path, value.workspacePath);
    documents[id] = { ...value, id, busy: false, saving: false, restored: true, revision: ++sequence };
  }
  const tabIds = new Map<string, string>();
  const tabs = (saved.tabs ?? []).filter(tab => tab && typeof tab.projectId === 'string' && typeof tab.path === 'string' && typeof tab.workspacePath === 'string' && ['file', 'diff'].includes(tab.mode)).map(tab => {
    const id = `${documentId(tab.projectId, tab.taskId, tab.path, tab.workspacePath)}:${tab.mode}`;
    tabIds.set(tab.id, id); return { ...tab, id };
  });
  const activeId = saved.activeId ? rebaseDesktopKey(saved.activeId, epoch) : null;
  const activeTabs = Object.fromEntries(Object.entries(saved.activeTabs ?? {}).flatMap(([scope, id]) => {
    const rebased = rebaseDesktopKey(scope, epoch), tab = tabIds.get(id);
    return rebased && tab ? [[rebased, tab]] : [];
  }));
  useDevelopmentEditor.setState(state => {
    const merged = { ...documents, ...state.documents };
    const active = state.activeId ?? (activeId && merged[activeId] ? activeId : null);
    return { documents: merged, tabs: [...tabs.filter(tab => !state.tabs.some(existing => existing.id === tab.id)), ...state.tabs], activeTabs: { ...activeTabs, ...state.activeTabs }, activeId: active, ...(active ? merged[active] : {}) };
  });
});
useDevelopmentEditor.subscribe((state, previous) => {
  if (state.documents === previous.documents && state.tabs === previous.tabs && state.activeTabs === previous.activeTabs && state.activeId === previous.activeId) return;
  setDesktopSlot('editor', { documents: Object.values(state.documents), tabs: state.tabs, activeId: state.activeId, activeTabs: state.activeTabs });
});

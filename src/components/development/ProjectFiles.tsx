import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, ContextMenu, Input, Modal, type MenuItem } from '@dgos/ui';
import { ChevronsDownUp, FilePlus2, FolderPlus, RefreshCw, Search } from 'lucide-react';
import { useLocale } from '../../i18n';
import { createDevelopmentEntry, inspectDevelopmentEntry, mutateDevelopmentEntry, openDevelopmentEntryDirectory, type InspectedEntry } from '../../stores/developmentExplorer';
import { listDevelopmentFiles, readDevelopmentFile, type DevFileEntry, type DevFileList } from '../../stores/developmentReview';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { developmentDraftKey, useDevelopmentDrafts, useScopedDevelopmentDraft } from '../../stores/developmentDrafts';
import { developmentEditorMutationAffects, guardDevelopmentDraft, guardDevelopmentDocuments, invalidateDevelopmentDocuments, remapDevelopmentDocuments, useDevelopmentEditor } from '../../stores/developmentEditor';
import type { ConversationReference } from '../../stores/developmentReferences';
import { useWorkbenchLocation } from './workbenchLocations';
import { FileTree } from './FileTree';
import { FileEditor } from './FileEditor';
import { confirmAction, errorText } from './shared';
import './workbench-tree.css';

export interface FileTarget { path: string; line?: number; mode: 'file' | 'diff'; nonce: number }
export interface FileMutation { action: 'rename' | 'delete'; path: string; destination: string | null }
interface Operation { path: string; kind: 'file' | 'directory'; name: string; status: 'pending' | 'unknown'; action?: 'copy' | 'rename' | 'delete'; destination?: string | null; version?: string }
interface FileDraft { name: string; operation: Operation | null }
const emptyDraft: FileDraft = { name: '', operation: null };
type Dialog = { action: 'file' | 'directory' | 'rename' | 'copy'; parent: string; source?: InspectedEntry };
const parentPath = (path: string) => path.split('/').slice(0, -1).join('/');
const joined = (parent: string, name: string) => [parent, name].filter(Boolean).join('/');
export const filePathAffected = (path: string, source: string) => path === source || path.startsWith(`${source}/`);
let clipboard: { scope: string; epoch: number; entry: InspectedEntry } | null = null;
useDevelopment.subscribe((state, previous) => { if (state.epoch !== previous.epoch) clipboard = null; });

export function Files({ projectId, taskId, workspacePath = '', target, onOpenFile, onReference, onMutation }: { projectId: string; taskId: string | null; workspacePath?: string; target?: FileTarget | null; onOpenFile?: (target: FileTarget) => void; onReference?: (reference: ConversationReference) => void; onMutation?: (mutation: FileMutation) => void }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const scope = JSON.stringify([projectId, taskId, workspacePath, 'files']);
  const [path, setPath] = useWorkbenchLocation(`${scope}:path`, '');
  const [query, setQuery] = useWorkbenchLocation(`${scope}:query`, '');
  const [ignored, setIgnored] = useWorkbenchLocation(`${scope}:ignored`, false);
  const [selected, setSelected] = useWorkbenchLocation<string | null>(`${scope}:selected`, null);
  const [opened, setOpened] = useWorkbenchLocation<string | null>(`${scope}:opened`, null);
  const [collapseKey, setCollapseKey] = useWorkbenchLocation(`${scope}:collapse`, 0);
  const [treeRefresh, setTreeRefresh] = useState(0);
  const [listing, setListing] = useState<DevFileList | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [, updateClipboard] = useState(0);
  const draft = useScopedDevelopmentDraft<FileDraft>('delivery', [projectId, taskId, workspacePath, 'file-create'], emptyDraft);
  const key = developmentDraftKey('delivery', projectId, taskId, workspacePath, 'file-create');
  const { operation, name } = draft.value;
  const mounted = useRef(true), request = useRef(0), working = useRef(false);
  const view = useRef({ scope });
  if (view.current.scope !== scope) { view.current = { scope }; working.current = false; }
  const currentScope = view.current;
  const valid = (generation: number, identity = currentScope) => mounted.current && view.current === identity && epoch === useDevelopment.getState().epoch && generation === contextGeneration();
  const latest = () => (useDevelopmentDrafts.getState().entries[key]?.value as FileDraft | undefined) ?? emptyDraft;
  const input = (entryPath: string) => ({ project_id: projectId, task_id: taskId, path: entryPath });
  const blocked = Boolean(operation);
  const copied = clipboard?.epoch === epoch && clipboard.scope === scope ? clipboard.entry : null;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++request.current; }; }, []);
  useEffect(() => { if (target) { setSelected(target.path); setOpened(target.path); setQuery(''); } }, [target?.nonce]);
  useEffect(() => { setDialog(null); setError(null); setListing(null); }, [scope, epoch]);
  const load = useCallback(async () => {
    const ticket = ++request.current, generation = contextGeneration();
    setLoading(true);
    const result = await listDevelopmentFiles({ project_id: projectId, task_id: taskId, path: '', query, include_ignored: ignored });
    if (ticket !== request.current || !valid(generation)) return;
    setLoading(false);
    if (result.ok) setListing(result.value); else setError(result.code);
  }, [projectId, taskId, workspacePath, query, ignored, epoch]);
  useEffect(() => { void load(); return () => { ++request.current; }; }, [load]);
  function finishMutation(op: Operation) {
    if (!op.action || op.action === 'copy') return;
    const mutation = { action: op.action, path: op.path, destination: op.destination ?? null };
    if (op.action === 'rename' && op.destination) remapDevelopmentDocuments(projectId, taskId, op.path, op.destination, workspacePath);
    else invalidateDevelopmentDocuments(projectId, taskId, [op.path], workspacePath);
    if (selected && filePathAffected(selected, op.path)) setSelected(op.action === 'rename' ? `${op.destination}${selected.slice(op.path.length)}` : null);
    if (opened && filePathAffected(opened, op.path)) setOpened(op.action === 'rename' ? `${op.destination}${opened.slice(op.path.length)}` : null);
    if (filePathAffected(path, op.path)) setPath(op.action === 'rename' ? `${op.destination}${path.slice(op.path.length)}` : parentPath(op.path));
    onMutation?.(mutation);
  }
  async function refreshFiles() {
    if (working.current || latest().operation?.status === 'pending') return;
    const generation = contextGeneration(), op = latest().operation;
    if (op) {
      const observed = await inspectDevelopmentEntry(input(op.path));
      if (!valid(generation) || latest().operation !== op) return;
      if (!observed.ok && observed.code !== 'file_not_found') { setError(observed.code); return; }
      if (!op.action) {
        const parent = await listDevelopmentFiles({ project_id: projectId, task_id: taskId, path: parentPath(op.path), query: '', include_ignored: true });
        if (!valid(generation) || latest().operation !== op) return;
        if (!parent.ok || parent.value.truncated) { setError(parent.ok ? 'scan_limit' : parent.code); return; }
        const exists = parent.value.entries.some(entry => entry.path === op.path);
        const detail = `${op.path}: ${observed.ok ? t('workbenchFiles.present') : t('workbenchFiles.absent')}\n\n${t('workbenchFiles.acceptObservedDetail')}`;
        if (!await confirmAction(t('workbenchFiles.inspectResult'), detail, t('workbenchFiles.acceptObserved')) || !valid(generation) || latest().operation !== op) return;
        if (exists && op.kind === 'file') open(op.path);
      } else {
        const destination = op.destination ? await inspectDevelopmentEntry(input(op.destination)) : null;
        if (!valid(generation) || latest().operation !== op) return;
        if (destination && !destination.ok && destination.code !== 'file_not_found') { setError(destination.code); return; }
        const sourceState = observed.ok ? t('workbenchFiles.present') : t('workbenchFiles.absent');
        const destinationState = destination ? destination.ok ? t('workbenchFiles.present') : t('workbenchFiles.absent') : '';
        const facts = `${op.path}: ${sourceState}${op.destination ? `\n${op.destination}: ${destinationState}` : ''}`;
        if (!await confirmAction(t('workbenchFiles.inspectResult'), `${facts}\n\n${t('workbenchFiles.acceptObservedDetail')}`, t('workbenchFiles.acceptObserved')) || !valid(generation) || latest().operation !== op) return;
        if (op.action !== 'copy' && (!await guardDevelopmentDraft(true) || !valid(generation) || latest().operation !== op)) return;
        if (op.action !== 'copy') invalidateDevelopmentDocuments(projectId, taskId, [op.path], workspacePath);
        if (op.action !== 'copy') {
          if (selected && filePathAffected(selected, op.path)) setSelected(null);
          if (opened && filePathAffected(opened, op.path)) setOpened(null);
          onMutation?.({ action: 'delete', path: op.path, destination: null });
        }
      }
      draft.setValue({ ...latest(), name: latest().name === op.name ? '' : latest().name, operation: null });
    }
    if (!valid(generation)) return;
    setError(null); setTreeRefresh(value => value + 1); await load();
  }
  async function inspect(entry: DevFileEntry): Promise<InspectedEntry | null> {
    const generation = contextGeneration();
    const result = await inspectDevelopmentEntry(input(entry.path));
    if (!valid(generation)) return null;
    if (!result.ok) { setError(result.code); return null; }
    return result.value;
  }
  async function attach(entry: DevFileEntry) {
    const generation = contextGeneration();
    setError(null);
    if (entry.kind === 'directory') { onReference?.({ ...input(entry.path), kind: 'path' }); return; }
    const result = await readDevelopmentFile(input(entry.path));
    if (!valid(generation)) return;
    if (!result.ok) { setError(result.code); return; }
    if (result.value.reason || result.value.text === null || !result.value.hash) { setError(result.value.reason ?? 'file_access_denied'); return; }
    onReference?.({ ...input(entry.path), kind: 'file', expected_hash: result.value.hash });
  }
  function beginCreate(kind: 'file' | 'directory', parent = path) {
    if (latest().operation || working.current) return;
    setError(null); setDialog({ action: kind, parent });
  }
  async function beginRename(entry: DevFileEntry) {
    if (latest().operation || working.current) return;
    working.current = true;
    const source = await inspect(entry);
    working.current = false;
    if (!source) return;
    draft.setValue({ ...latest(), name: entry.name });
    setDialog({ action: 'rename', parent: parentPath(entry.path), source });
  }
  async function copy(entry: DevFileEntry) {
    if (latest().operation || working.current) return;
    working.current = true;
    const source = await inspect(entry);
    working.current = false;
    if (source) { clipboard = { scope, epoch, entry: source }; updateClipboard(value => value + 1); setError(null); }
  }
  async function paste(parent: string) {
    if (!copied || latest().operation || working.current) return;
    const basename = copied.path.split('/').pop()!;
    const same = parentPath(copied.path) === parent;
    const dot = copied.kind === 'file' ? basename.lastIndexOf('.') : -1;
    const copyName = dot > 0 ? `${basename.slice(0, dot)} copy${basename.slice(dot)}` : `${basename} copy`;
    draft.setValue({ ...latest(), name: same ? copyName : basename });
    setError(null); setDialog({ action: 'copy', parent, source: copied });
  }
  async function mutate(action: 'copy' | 'rename' | 'delete', source: InspectedEntry, destination: string | null) {
    if (working.current || latest().operation) return;
    const generation = contextGeneration(), identity = currentScope;
    working.current = true; setError(null);
    const documents = Object.values(useDevelopmentEditor.getState().documents).filter(document => document.projectId === projectId && document.taskId === taskId && document.workspacePath === workspacePath && filePathAffected(document.path, source.path));
    if (developmentEditorMutationAffects(projectId, taskId, source.path) || action !== 'copy' && documents.some(document => document.saving || document.busy)) { setError('workspace_busy'); working.current = false; return; }
    if (action === 'delete' && !await guardDevelopmentDocuments(projectId, taskId, [source.path], workspacePath)) { working.current = false; return; }
    if (!valid(generation, identity) || latest().operation || action !== 'copy' && documents.some(document => useDevelopmentEditor.getState().documents[document.id] !== document)) { working.current = false; return; }
    const pending: Operation = { ...source, name, action, destination, status: 'pending' };
    draft.setValue({ ...latest(), operation: pending });
    const result = await mutateDevelopmentEntry({ ...input(source.path), expected_version: source.version, action, destination, trust_directory: true });
    working.current = false;
    if (epoch !== useDevelopment.getState().epoch || latest().operation !== pending) return;
    const unknown = !result.ok && ['file_mutation_unknown', 'unknown'].includes(result.code);
    const staleSuccess = result.ok && !valid(generation, identity);
    draft.setValue({ ...latest(), operation: unknown || staleSuccess ? { ...pending, status: 'unknown' } : null });
    if (!valid(generation, identity)) return;
    if (!result.ok) { setError(result.code); return; }
    finishMutation(pending); setDialog(null); draft.setValue({ ...latest(), name: '' });
    setTreeRefresh(value => value + 1); await load();
  }
  async function remove(entry: DevFileEntry) {
    if (working.current || latest().operation) return;
    const generation = contextGeneration();
    working.current = true;
    const source = await inspect(entry);
    if (!source) { working.current = false; return; }
    const confirmed = await confirmAction(t('workbenchFiles.delete'), t('workbenchFiles.deleteConfirm', { path: entry.path }), t('workbenchFiles.delete'));
    working.current = false;
    if (confirmed && valid(generation)) await mutate('delete', source, null);
  }
  async function submit() {
    if (!dialog || blocked || working.current) return;
    const cleanName = name.trim();
    if (!cleanName || ['.', '..'].includes(cleanName) || /[\\/\x00]/.test(cleanName)) { setError('invalid_request'); return; }
    const destination = joined(dialog.parent, cleanName);
    if (dialog.source) { await mutate(dialog.action === 'rename' ? 'rename' : 'copy', dialog.source, destination); return; }
    const generation = contextGeneration(), identity = currentScope;
    working.current = true;
    if (!await confirmAction(t('workbenchFiles.createTrust'), t('workbenchFiles.createTrustDetail', { path: destination }), t(dialog.action === 'file' ? 'workbenchFiles.newFile' : 'workbenchFiles.newFolder')) || !valid(generation, identity)) { working.current = false; return; }
    const pending: Operation = { path: destination, kind: dialog.action as 'file' | 'directory', name, status: 'pending' };
    draft.setValue({ ...latest(), operation: pending });
    const result = await createDevelopmentEntry({ ...input(destination), kind: pending.kind, trust_directory: true });
    working.current = false;
    if (epoch !== useDevelopment.getState().epoch || latest().operation !== pending) return;
    const unknown = !result.ok && ['file_create_unknown', 'unknown'].includes(result.code);
    draft.setValue({ ...latest(), name: result.ok && latest().name === name ? '' : latest().name, operation: unknown ? { ...pending, status: 'unknown' } : null });
    if (!valid(generation, identity)) return;
    if (!result.ok) { setError(result.code); return; }
    setDialog(null); setError(null); setTreeRefresh(value => value + 1); await load();
    if (pending.kind === 'file' && valid(generation, identity)) open(destination);
  }
  async function reveal(entry: DevFileEntry) {
    const generation = contextGeneration();
    const result = await openDevelopmentEntryDirectory(input(entry.path));
    if (valid(generation) && !result.ok) setError(result.code);
  }
  function open(entryPath: string) {
    setSelected(entryPath); setPath(parentPath(entryPath));
    if (onOpenFile) { onOpenFile({ path: entryPath, mode: 'file', nonce: Date.now() }); return; }
    setOpened(entryPath);
  }
  const shortcut = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl+';
  function menu(entry: DevFileEntry): MenuItem[] {
    const disabled = blocked || !['file', 'directory'].includes(entry.kind);
    return [
      { kind: 'item', id: 'attach', label: t('workbenchFiles.addConversation'), icon: 'message-square-plus', disabled: disabled || !onReference, onSelect: () => void attach(entry) },
      ...(entry.kind === 'directory' ? [{ kind: 'separator' } as MenuItem, { kind: 'item', id: 'new-file', label: t('workbenchFiles.newFile'), icon: 'file-plus', disabled, onSelect: () => beginCreate('file', entry.path) } as MenuItem, { kind: 'item', id: 'new-folder', label: t('workbenchFiles.newFolder'), icon: 'folder-plus', disabled, onSelect: () => beginCreate('directory', entry.path) } as MenuItem] : []),
      { kind: 'separator' },
      { kind: 'item', id: 'copy', label: t('workbenchFiles.copy'), icon: 'copy', shortcut: `${shortcut}C`, disabled, onSelect: () => void copy(entry) },
      { kind: 'item', id: 'paste', label: t('workbenchFiles.paste'), icon: 'clipboard-paste', shortcut: `${shortcut}V`, disabled: disabled || !copied, onSelect: () => void paste(entry.kind === 'directory' ? entry.path : parentPath(entry.path)) },
      { kind: 'separator' },
      { kind: 'item', id: 'rename', label: t('workbenchFiles.rename'), icon: 'pencil', disabled, onSelect: () => void beginRename(entry) },
      { kind: 'item', id: 'reveal', label: t('workbenchFiles.openDirectory'), icon: 'folder-open', disabled, onSelect: () => void reveal(entry) },
      { kind: 'separator' },
      { kind: 'item', id: 'delete', label: t('workbenchFiles.delete'), icon: 'trash-2', danger: true, disabled, onSelect: () => void remove(entry) },
    ];
  }
  const background: MenuItem[] = [
    { kind: 'item', id: 'new-file', label: t('workbenchFiles.newFile'), disabled: blocked, onSelect: () => beginCreate('file') },
    { kind: 'item', id: 'new-folder', label: t('workbenchFiles.newFolder'), disabled: blocked, onSelect: () => beginCreate('directory') },
    { kind: 'item', id: 'paste', label: t('workbenchFiles.paste'), disabled: blocked || !copied, onSelect: () => void paste(path) },
    { kind: 'separator' },
    { kind: 'checkbox', id: 'ignored', label: t('review.includeIgnored'), checked: ignored, onCheckedChange: setIgnored },
  ];
  return <div className="dev-review-stack dev-file-explorer" onKeyDown={event => {
    if (event.target === event.currentTarget || (event.target as HTMLElement).closest('input,textarea,[contenteditable=true],[role=dialog],[role=menu]')) return;
    const node = (event.target as HTMLElement).closest<HTMLElement>('[data-file-path]');
    const entry = node ? { path: node.dataset.filePath!, name: node.dataset.filePath!.split('/').pop()!, kind: node.dataset.fileKind as DevFileEntry['kind'], size: 0, ignored: false } : null;
    if (!entry || blocked) return;
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'c') { event.preventDefault(); void copy(entry); }
    else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'v') { event.preventDefault(); void paste(entry.kind === 'directory' ? entry.path : parentPath(entry.path)); }
    else if (event.key === 'F2') { event.preventDefault(); void beginRename(entry); }
    else if (event.key === 'Delete') { event.preventDefault(); void remove(entry); }
  }}>
    <ContextMenu items={background}><div className="dev-explorer-toolbar">
      <label className="dev-explorer-search"><Search size={15} aria-hidden="true" /><input type="search" data-testid="dev-file-search" aria-label={t('review.search')} placeholder={t('workbenchFiles.filterPlaceholder')} value={query} onChange={event => setQuery(event.target.value)} /></label>
      <Button size="xs" variant="ghost" aria-label={t('workbenchFiles.newFile')} title={t('workbenchFiles.newFile')} disabled={blocked} onClick={() => beginCreate('file')}><FilePlus2 size={16} aria-hidden="true" /></Button>
      <Button size="xs" variant="ghost" aria-label={t('workbenchFiles.newFolder')} title={t('workbenchFiles.newFolder')} disabled={blocked} onClick={() => beginCreate('directory')}><FolderPlus size={16} aria-hidden="true" /></Button>
      <Button size="xs" variant="ghost" data-testid="dev-review-refresh" aria-label={t('review.refresh')} title={t('review.refresh')} disabled={loading || operation?.status === 'pending'} onClick={() => void refreshFiles()}><RefreshCw size={16} aria-hidden="true" /></Button>
      <Button size="xs" variant="ghost" aria-label={t('workbenchFiles.collapseAll')} title={t('workbenchFiles.collapseAll')} onClick={() => setCollapseKey(value => value + 1)}><ChevronsDownUp size={16} aria-hidden="true" /></Button>
    </div></ContextMenu>
    {loading && !listing ? <p role="status">{t('review.loading')}</p> : null}
    {error ? <p role="alert" className="dev-explorer-feedback">{errorText(error)}</p> : null}
    {operation ? <p role="status" className="dev-explorer-feedback">{operation.status === 'pending' ? t('review.loading') : errorText(operation.action ? 'file_mutation_unknown' : 'file_create_unknown')} <code>{operation.path}</code></p> : null}
    {listing?.truncated ? <p role="status" className="dev-explorer-feedback">{t('review.filesTruncated')}</p> : null}
    {listing ? <div className={onOpenFile ? 'dev-file-browser' : 'dev-file-layout'}><nav aria-label={t('review.files')} className="dev-file-list">
      {!listing.entries.length ? <p className="text-xs text-ink-secondary">{t('review.noFiles')}</p> : null}
      <FileTree projectId={projectId} taskId={taskId} scope={scope} entries={listing.entries} query={query} ignored={ignored} selected={selected} refreshKey={treeRefresh} collapseKey={collapseKey} onOpen={open} onDirectory={setPath} onSelect={setSelected} menuItems={menu} />
    </nav>{!onOpenFile ? <div className="min-w-0">{opened ? <FileEditor key={opened} projectId={projectId} taskId={taskId} workspacePath={workspacePath} path={opened} line={target?.path === opened ? target.line : undefined} onReference={onReference} /> : <p className="text-xs text-ink-secondary">{t('review.selectFile')}</p>}</div> : null}</div> : null}
    <Modal open={Boolean(dialog)} onOpenChange={open => { if (!open && !latest().operation) setDialog(null); }} title={t(dialog?.action === 'rename' ? 'workbenchFiles.rename' : dialog?.action === 'copy' ? 'workbenchFiles.paste' : dialog?.action === 'directory' ? 'workbenchFiles.newFolder' : 'workbenchFiles.newFile')} footer={<><Button variant="secondary" disabled={blocked} onClick={() => setDialog(null)}>{t('dev.cancel')}</Button><Button disabled={blocked || !name.trim()} onClick={() => void submit()}>{t(dialog?.action === 'rename' ? 'workbenchFiles.rename' : dialog?.action === 'copy' ? 'workbenchFiles.paste' : 'workbenchFiles.create')}</Button></>}>
      <Input label={t('workbenchFiles.entryName')} value={name} disabled={blocked} onChange={event => draft.setValue({ ...latest(), name: event.target.value })} onKeyDown={event => { if (event.key === 'Enter' && !event.nativeEvent.isComposing) { event.preventDefault(); void submit(); } }} />
      {dialog?.parent ? <p className="text-xs text-ink-secondary">{dialog.parent}/</p> : null}
      {error ? <p role="alert">{errorText(error)}</p> : null}
      {operation?.status === 'unknown' ? <Button variant="secondary" onClick={() => void refreshFiles()}>{t('review.refresh')}</Button> : null}
    </Modal>
  </div>;
}

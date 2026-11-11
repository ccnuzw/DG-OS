import { Fragment, useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import { Button, DropdownMenu } from '@dgos/ui';
import { ChevronDown, Ellipsis, Pencil, Plus } from 'lucide-react';
import { common, createLowlight } from 'lowlight';
import { useLocale } from '../../i18n';
import { useDevelopmentEditor, documentId, documentDirty, developmentEditorMutationAffects, cancelDevelopmentDocumentEdit } from '../../stores/developmentEditor';
import { useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { errorText } from './shared';
import { ManualTakeover } from './ManualTakeover';
import type { ConversationReference } from '../../stores/developmentReferences';
import Markdown from '../chat/Markdown';
import './file-workspace.css';

const lowlight = createLowlight(common);
const languages: Record<string, string> = { ts: 'typescript', tsx: 'typescript', js: 'javascript', jsx: 'javascript', rs: 'rust', py: 'python', sh: 'bash', yml: 'yaml', md: 'markdown' };
function syntaxNodes(nodes: readonly unknown[]): ReactNode[] {
  return nodes.map((node, index) => {
    if (!node || typeof node !== 'object' || !('type' in node)) return null;
    if (node.type === 'text' && 'value' in node && typeof node.value === 'string') return <Fragment key={index}>{node.value}</Fragment>;
    if (node.type !== 'element' || !('children' in node) || !Array.isArray(node.children)) return null;
    const properties = 'properties' in node && node.properties && typeof node.properties === 'object' ? node.properties : null;
    const classes = properties && 'className' in properties && Array.isArray(properties.className) ? properties.className.filter((name): name is string => typeof name === 'string' && /^hljs-[a-z_-]+$/.test(name)) : [];
    return <span key={index} className={classes.join(' ') || undefined}>{syntaxNodes(node.children)}</span>;
  });
}
export function FileEditor({ projectId, taskId, path, line, workspacePath = '', visible = true, onReference, editing: controlledEditing, onEditingChange, onPrepareEdit }: { projectId: string; taskId: string | null; path: string; line?: number; workspacePath?: string; visible?: boolean; onReference?: (reference: ConversationReference) => void; editing?: boolean; onEditingChange?: (editing: boolean) => void; onPrepareEdit?: () => Promise<boolean> }) {
  const { t } = useLocale();
  const id = documentId(projectId, taskId, path, workspacePath);
  const editor = useDevelopmentEditor(state => state.documents[id]);
  useEffect(() => { void useDevelopmentEditor.getState().open(projectId, taskId, path, workspacePath); }, [projectId, taskId, path, workspacePath]);
  const dirty = editor ? documentDirty(editor) : false;
  const mutationBlocked = useDevelopmentDrafts(() => developmentEditorMutationAffects(projectId, taskId, path));
  const file = editor?.file;
  const textarea = useRef<HTMLTextAreaElement>(null), gutter = useRef<HTMLPreElement>(null);
  const errorId = useId();
  const positioned = useRef<string | null>(null);
  const [selection, setSelection] = useState<{ start: number; end: number } | null>(null);
  const [localEditingId, setLocalEditingId] = useState<string | null>(null);
  const editing = controlledEditing ?? localEditingId === id;
  const [preparing, setPreparing] = useState(false);
  const [prepareFailed, setPrepareFailed] = useState(false);
  const [attaching, setAttaching] = useState(false);
  const operation = useRef(false);
  const identity = useRef(id);
  identity.current = id;
  useEffect(() => { identity.current = id; return () => { identity.current = ''; }; }, [id]);
  useEffect(() => { setSelection(null); setPrepareFailed(false); setPreparing(false); setAttaching(false); operation.current = false; }, [id]);
  function setEditing(value: boolean) { setLocalEditingId(value ? id : null); onEditingChange?.(value); }
  const markdown = /\.(md|markdown|mdown)$/i.test(path);
  const [preview, setPreview] = useState(markdown && line === undefined);
  useEffect(() => { setPreview(markdown && line === undefined && !editing); }, [id, line, markdown]);
  useEffect(() => {
    if (!editing || !visible || editor?.busy) return;
    setPreview(false);
    const frame = window.requestAnimationFrame(() => textarea.current?.focus({ preventScroll: true }));
    return () => window.cancelAnimationFrame(frame);
  }, [editing, visible, editor?.busy]);
  const highlighted = useMemo(() => {
    if (!preview || markdown || !editor || editor.draft.length > 20000) return null;
    const extension = path.slice(path.lastIndexOf('.') + 1).toLowerCase();
    const language = languages[extension] ?? extension;
    if (!lowlight.registered(language)) return null;
    try { return syntaxNodes(lowlight.highlight(language, editor.draft).children); } catch { return null; }
  }, [preview, markdown, path, editor?.draft]);
  const invalidLine = line !== undefined && file?.text != null && (!Number.isSafeInteger(line) || line < 1 || line > file.text.split('\n').length);
  const readUnavailable = !editor || editor.busy || editor.saving || mutationBlocked || preparing || attaching;
  const unavailable = readUnavailable || Boolean(editor?.restored);
  const canSave = Boolean(taskId && dirty && file?.hash && file.text !== null && !unavailable && editor?.error !== 'file_changed');
  async function beginEditing() {
    if (operation.current || unavailable) return;
    operation.current = true; setPreparing(true); setPrepareFailed(false);
    try {
      const ready = onPrepareEdit ? await onPrepareEdit() : Boolean(taskId);
      if (identity.current !== id) return;
      if (ready) { setEditing(true); setPreview(false); } else setPrepareFailed(true);
    } catch { if (identity.current === id) setPrepareFailed(true); }
    finally { operation.current = false; if (identity.current === id) setPreparing(false); }
  }
  async function cancelEditing() {
    if (operation.current || unavailable) return;
    operation.current = true;
    try {
      if (!await cancelDevelopmentDocumentEdit(id) || identity.current !== id) return;
      setSelection(null); setEditing(false);
    } finally { operation.current = false; }
  }
  async function attach(kind: 'file' | 'selection' | 'path') {
    if (operation.current || unavailable || !onReference || kind === 'selection' && !selection) return;
    operation.current = true; setAttaching(true);
    const selected = selection;
    try {
      if (kind !== 'path' && dirty) {
        if (!canSave) return;
        await useDevelopmentEditor.getState().save(id);
      }
      const current = useDevelopmentEditor.getState().documents[id];
      if (identity.current !== id || !current || current.busy || current.saving || current.restored) return;
      if (kind !== 'path' && (current.error || documentDirty(current) || !current.file?.hash || current.file.text === null)) return;
      onReference({ project_id: projectId, task_id: taskId, path, kind, ...(kind !== 'path' ? { expected_hash: current.file!.hash! } : {}), ...(kind === 'selection' && selected ? { start_line: selected.start, end_line: selected.end } : {}) });
    } finally { operation.current = false; if (identity.current === id) setAttaching(false); }
  }
  useEffect(() => {
    const node = textarea.current, target = `${id}:${line}`;
    if (!visible || preview || !node || !line || !Number.isSafeInteger(line) || line < 1 || editor?.busy || file?.text == null || positioned.current === target) return;
    const lines = editor?.draft.split('\n') ?? [];
    if (line > lines.length) return;
    let start = 0;
    for (let index = 0; index < line - 1; index++) start += lines[index].length + 1;
    node.focus({ preventScroll: true }); node.setSelectionRange(start, start + lines[line - 1].length);
    const lineHeight = Number.parseFloat(window.getComputedStyle(node).lineHeight) || 20;
    node.scrollTop = Math.max(0, (line - 1) * lineHeight - node.clientHeight / 2);
    if (gutter.current) gutter.current.scrollTop = node.scrollTop;
    positioned.current = target;
  }, [id, line, visible, preview, editor?.busy, editor?.draft, file]);
  return <article className="file-editor" data-testid="dev-file-source" onKeyDown={event => {
    if (editing && visible && (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's' && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) {
      event.preventDefault(); if (!event.repeat && canSave) void useDevelopmentEditor.getState().save(id);
    }
  }}>
    <header className="file-editor-toolbar">
      <div className="file-editor-title"><strong title={path}>{path.split('/').pop()}</strong><span title={path}>{path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : t('workbenchFiles.projectRoot')}</span></div>
      <div className="file-editor-actions">
        {editing ? <><Button size="sm" variant="ghost" disabled={unavailable} onClick={() => void cancelEditing()}>{t('common.cancel')}</Button><Button size="sm" data-testid="dev-file-save" disabled={!canSave} onClick={() => void useDevelopmentEditor.getState().save(id)}>{t(editor?.saving ? 'direct.saving' : 'workbenchFiles.save')}</Button></> : <Button size="sm" data-testid="dev-file-edit" variant="secondary" disabled={unavailable || !file?.hash || file.text === null || (!taskId && !onPrepareEdit)} onClick={() => void beginEditing()}><Pencil size={14} aria-hidden="true" />{t(preparing ? 'workbenchFiles.openingEditor' : 'workbenchFiles.editFile')}</Button>}
        <DropdownMenu label={t('workbenchFiles.fileActions')} items={[{ kind: 'label', text: t('workbenchFiles.reloadHint') }, { kind: 'item', id: 'reload', label: t('workbenchFiles.reloadDisk'), disabled: readUnavailable, onSelect: () => { void useDevelopmentEditor.getState().reloadDisk(id); setSelection(null); } }]} trigger={<Button size="sm" variant="ghost" aria-label={t('workbenchFiles.fileActions')}><Ellipsis size={16} aria-hidden="true" /></Button>} />
      </div>
    </header>
    <div className="file-editor-viewbar"><div role="group" aria-label={t('workbenchFiles.displayMode')}><Button size="xs" variant={!preview ? 'secondary' : 'ghost'} aria-pressed={!preview} onClick={() => setPreview(false)}>{t('workbenchFiles.source')}</Button><Button size="xs" variant={preview ? 'secondary' : 'ghost'} aria-pressed={preview} onClick={() => { setPreview(true); setSelection(null); }}>{t('fileWorkspace.preview')}</Button></div><span role="status">{t(editor?.saving ? 'direct.saving' : dirty ? 'direct.unsaved' : editing ? 'direct.saved' : 'workbenchFiles.browsing')}</span></div>
    <div className="file-editor-body">
      {prepareFailed ? <p role="alert" className="dev-feedback dev-feedback-error">{t('workbenchFiles.editFailed')}</p> : null}
      {invalidLine ? <p role="alert" className="dev-evidence-warning">{t('workbenchFiles.invalidLine')}</p> : null}
      {!editor || editor.busy ? <p role="status">{t('review.loading')}</p> : null}
      {editor?.error && editor.error !== 'file_changed' ? <p id={errorId} role="alert" className="dev-feedback dev-feedback-error">{path}: {errorText(editor.error)}</p> : null}
      {editor?.restored ? <p role="status" className="dev-evidence-warning">{t('direct.recoveredDraft')}</p> : null}
      {editor?.error === 'workspace_busy' && taskId ? <ManualTakeover projectId={projectId} taskId={taskId} /> : null}
      {file && editor ? file.text !== null ? <>
        <div className="file-editor-content" hidden={preview}><pre ref={gutter} className="file-editor-gutter" aria-hidden="true">{editor.draft.split('\n').map((_, index) => index + 1).join('\n')}</pre><textarea ref={textarea} className="dev-editor" data-testid="dev-file-editor" aria-label={t('direct.source')} aria-invalid={Boolean(editor.error)} aria-describedby={editor.error ? errorId : undefined} spellCheck={false} wrap="off" value={editor.draft} readOnly={!editing || !taskId || mutationBlocked || attaching || editor.saving} onScroll={event => { if (gutter.current) gutter.current.scrollTop = event.currentTarget.scrollTop; }} onSelect={event => { const node = event.currentTarget; setSelection(node.selectionStart === node.selectionEnd ? null : { start: node.value.slice(0, node.selectionStart).split('\n').length, end: node.value.slice(0, Math.max(node.selectionStart, node.selectionEnd - 1)).split('\n').length }); }} onChange={event => { if (!editing || !taskId || unavailable) return; useDevelopmentEditor.getState().edit(event.target.value, id); setSelection(null); }} /></div>
        {preview ? <section className="file-editor-preview" aria-label={t('fileWorkspace.preview')}>{markdown ? <Markdown source={editor.draft} /> : <pre className="file-code-preview chat-code"><code>{highlighted ?? editor.draft}</code></pre>}</section> : null}
      </> : <p className="text-xs">{t(`review.reason.${file.reason ?? 'unavailable'}`)}</p> : null}
      {editor?.error === 'file_changed' ? <section className="dev-review-stack" aria-label={t('direct.conflict')} data-testid="dev-file-conflict"><p id={errorId} role="alert" className="dev-evidence-warning">{t('workbenchFiles.externalChange')}</p><div className="dev-comparison"><div><h3 className="text-xs">{t('direct.localDraft')}</h3><pre className="dev-diff" tabIndex={0}>{editor.draft}</pre></div><div><h3 className="text-xs">{t('direct.diskContent')}</h3><pre className="dev-diff" tabIndex={0}>{editor.disk?.text ?? t('direct.diskUnavailable')}</pre></div></div><Button size="xs" variant="secondary" data-testid="dev-file-reload" disabled={editor.saving} onClick={() => void useDevelopmentEditor.getState().reloadDisk(id)}>{t('workbenchFiles.reloadDisk')}</Button></section> : null}
    </div>
    {onReference && file ? <footer className="file-editor-reference"><div className="file-editor-reference-copy"><strong>{selection ? t('workbenchFiles.selectedLines', { start: selection.start, end: selection.end }) : t('workbenchFiles.referenceTitle')}</strong><p>{t(dirty ? 'workbenchFiles.saveBeforeReference' : 'workbenchFiles.referenceHint')}</p></div><div className="file-editor-reference-actions"><Button size="sm" variant={editing ? 'secondary' : 'primary'} disabled={unavailable || !file.hash || file.text === null || Boolean(editor?.error) || dirty && !canSave} onClick={() => void attach(selection ? 'selection' : 'file')}><Plus size={16} aria-hidden="true" />{t(attaching ? 'workbenchFiles.adding' : dirty ? 'workbenchFiles.saveAndAdd' : selection ? 'workbenchFiles.addSelection' : 'workbenchFiles.addConversation')}</Button><DropdownMenu label={t('workbenchFiles.addOptions')} items={[
      { kind: 'item', id: 'file', label: t(dirty ? 'workbenchFiles.saveAndAddFile' : 'workbenchFiles.wholeFile'), disabled: unavailable || !file.hash || file.text === null || Boolean(editor?.error) || dirty && !canSave, onSelect: () => { void attach('file'); } },
      ...(selection ? [{ kind: 'item' as const, id: 'selection', label: t(dirty ? 'workbenchFiles.saveAndAddSelection' : 'workbenchFiles.addSelection'), disabled: unavailable || Boolean(editor?.error) || dirty && !canSave, onSelect: () => { void attach('selection'); } }] : []),
      { kind: 'item', id: 'path', label: t('workbenchFiles.pathOnly'), disabled: unavailable, onSelect: () => { void attach('path'); } },
    ]} trigger={<Button size="sm" variant="secondary" aria-label={t('workbenchFiles.addOptions')}><ChevronDown size={16} aria-hidden="true" /></Button>} /></div></footer> : null}
  </article>;
}

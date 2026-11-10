import { Fragment, useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import { Button } from '@dgos/ui';
import { Code2, Eye } from 'lucide-react';
import { common, createLowlight } from 'lowlight';
import { useLocale } from '../../i18n';
import { useDevelopmentEditor, documentId, documentDirty, developmentEditorMutationAffects } from '../../stores/developmentEditor';
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
export function FileEditor({ projectId, taskId, path, line, workspacePath = '', visible = true, onReference }: { projectId: string; taskId: string | null; path: string; line?: number; workspacePath?: string; visible?: boolean; onReference?: (reference: ConversationReference) => void }) {
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
  const markdown = /\.(md|markdown|mdown)$/i.test(path);
  const [preview, setPreview] = useState(markdown && line === undefined);
  useEffect(() => { setPreview(markdown && line === undefined); }, [id, line, markdown]);
  const highlighted = useMemo(() => {
    if (!preview || markdown || !editor || editor.draft.length > 20000) return null;
    const extension = path.slice(path.lastIndexOf('.') + 1).toLowerCase();
    const language = languages[extension] ?? extension;
    if (!lowlight.registered(language)) return null;
    try { return syntaxNodes(lowlight.highlight(language, editor.draft).children); } catch { return null; }
  }, [preview, markdown, path, editor?.draft]);
  const invalidLine = line !== undefined && file?.text != null && (!Number.isSafeInteger(line) || line < 1 || line > file.text.split('\n').length);
  function attach(selected: boolean) {
    if (!file?.hash || file.text === null || dirty || editor?.busy || editor?.saving || !onReference || selected && !selection) return;
    onReference({ project_id: projectId, task_id: taskId, path, kind: selected ? 'selection' : 'file', expected_hash: file.hash, ...(selected && selection ? { start_line: selection.start, end_line: selection.end } : {}) });
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
  return <article className="dev-review-stack file-editor" data-testid="dev-file-source">
    <div className="dev-review-toolbar file-editor-toolbar"><strong className="break-all text-xs">{path}</strong><span role="status" className="text-xs">{t(editor?.saving ? 'direct.saving' : dirty ? 'direct.unsaved' : 'direct.saved')}</span><Button size="xs" variant="ghost" aria-label={t(preview ? 'direct.source' : 'fileWorkspace.preview')} title={t(preview ? 'direct.source' : 'fileWorkspace.preview')} aria-pressed={preview} onClick={() => setPreview(value => !value)}>{preview ? <Code2 size={16} aria-hidden="true" /> : <Eye size={16} aria-hidden="true" />}</Button></div>
    {invalidLine ? <p role="alert" className="dev-evidence-warning">{t('workbenchFiles.invalidLine')}</p> : null}
    {!editor || editor.busy ? <p role="status">{t('review.loading')}</p> : null}
    {editor?.error ? <p id={errorId} role="alert" className="dev-feedback dev-feedback-error">{path}: {errorText(editor.error)}</p> : null}
    {editor?.error === 'workspace_busy' && taskId ? <ManualTakeover projectId={projectId} taskId={taskId} /> : null}
    {file && editor ? <>
      {file.text !== null ? <>
        <div className="file-editor-content" hidden={preview}><pre ref={gutter} className="file-editor-gutter" aria-hidden="true">{editor.draft.split('\n').map((_, index) => index + 1).join('\n')}</pre><textarea ref={textarea} className="dev-editor" data-testid="dev-file-editor" aria-label={t('direct.source')} aria-invalid={Boolean(editor.error)} aria-describedby={editor.error ? errorId : undefined} spellCheck={false} wrap="off" value={editor.draft} readOnly={!taskId || mutationBlocked} onScroll={event => { if (gutter.current) gutter.current.scrollTop = event.currentTarget.scrollTop; }} onSelect={event => { const node = event.currentTarget; setSelection(node.selectionStart === node.selectionEnd ? null : { start: node.value.slice(0, node.selectionStart).split('\n').length, end: node.value.slice(0, Math.max(node.selectionStart, node.selectionEnd - 1)).split('\n').length }); }} onChange={event => { useDevelopmentEditor.getState().edit(event.target.value, id); setSelection(null); }} onKeyDown={event => { if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's' && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) { event.preventDefault(); if (!event.repeat) void useDevelopmentEditor.getState().save(id); } }} /></div>
        {preview ? <section className="file-editor-preview" aria-label={t('fileWorkspace.preview')}>{markdown ? <Markdown source={editor.draft} /> : <pre className="file-code-preview chat-code"><code>{highlighted ?? editor.draft}</code></pre>}</section> : null}
        {!taskId ? <p className="dev-evidence-warning">{t('direct.taskRequired')}</p> : null}
        <div className="flex flex-wrap gap-2"><Button size="xs" data-testid="dev-file-save" disabled={!taskId || !dirty || editor.saving || mutationBlocked || editor.error === 'file_changed'} onClick={() => void useDevelopmentEditor.getState().save(id)}>{t('direct.save')}</Button><Button size="xs" variant="secondary" disabled={editor.saving || mutationBlocked} onClick={() => void useDevelopmentEditor.getState().reloadDisk(id)}>{t('direct.reload')}</Button></div>
      </> : <p className="text-xs">{t(`review.reason.${file.reason ?? 'unavailable'}`)}</p>}
      {onReference ? <div className="dev-review-toolbar"><Button size="xs" variant="secondary" disabled={dirty || editor.busy || editor.saving || !file.hash} onClick={() => attach(false)}>{t('workbenchFiles.referenceFile')}</Button><Button size="xs" variant="secondary" disabled={dirty || editor.busy || editor.saving || !file.hash || !selection} onClick={() => attach(true)}>{t('workbenchFiles.referenceSelection')}</Button>{dirty ? <p className="text-xs">{t('workbenchFiles.saveBeforeReference')}</p> : null}</div> : null}
    </> : null}
    {editor?.error === 'file_changed' ? <section className="dev-review-stack" aria-label={t('direct.conflict')} data-testid="dev-file-conflict"><p role="alert" className="dev-evidence-warning">{t('direct.conflict')}</p><div className="dev-comparison"><div><h3 className="text-xs">{t('direct.localDraft')}</h3><pre className="dev-diff" tabIndex={0}>{editor.draft}</pre></div><div><h3 className="text-xs">{t('direct.diskContent')}</h3><pre className="dev-diff" tabIndex={0}>{editor.disk?.text ?? t('direct.diskUnavailable')}</pre></div></div><Button size="xs" variant="secondary" data-testid="dev-file-reload" disabled={editor.saving} onClick={() => void useDevelopmentEditor.getState().reloadDisk(id)}>{t('direct.reloadDiscard')}</Button></section> : null}
  </article>;
}

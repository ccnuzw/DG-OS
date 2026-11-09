import { useEffect, useId, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopmentEditor, hasDevelopmentDraft, developmentEditorMutationAffects } from '../../stores/developmentEditor';
import { useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { errorText } from './shared';
import { ManualTakeover } from './ManualTakeover';
import type { ConversationReference } from '../../stores/developmentReferences';

export function FileEditor({ projectId, taskId, path, line, visible = true, onReference }: { projectId: string; taskId: string | null; path: string; line?: number; visible?: boolean; onReference?: (reference: ConversationReference) => void }) {
  const { t } = useLocale();
  const editor = useDevelopmentEditor();
  useEffect(() => { void useDevelopmentEditor.getState().open(projectId, taskId, path); }, [projectId, taskId, path]);
  const dirty = hasDevelopmentDraft();
  const mutationBlocked = useDevelopmentDrafts(() => developmentEditorMutationAffects(projectId, taskId, path));
  const file = editor.projectId === projectId && editor.taskId === taskId && editor.path === path ? editor.file : null;
  const textarea = useRef<HTMLTextAreaElement>(null);
  const errorId = useId();
  const positioned = useRef<string | null>(null);
  const [selection, setSelection] = useState<{ start: number; end: number } | null>(null);
  const invalidLine = line !== undefined && file?.text != null && (!Number.isSafeInteger(line) || line < 1 || line > file.text.split('\n').length);
  function attach(selected: boolean) {
    if (!file?.hash || file.text === null || dirty || editor.busy || editor.saving || !onReference || selected && !selection) return;
    onReference({ project_id: projectId, task_id: taskId, path, kind: selected ? 'selection' : 'file', expected_hash: file.hash, ...(selected && selection ? { start_line: selection.start, end_line: selection.end } : {}) });
  }
  useEffect(() => {
    const node = textarea.current;
    const target = `${projectId}:${taskId}:${path}:${line}`;
    if (!visible || !node || !line || !Number.isSafeInteger(line) || line < 1 || editor.busy || file?.text == null || editor.projectId !== projectId || editor.taskId !== taskId || editor.path !== path || positioned.current === target) return;
    const lines = editor.draft.split('\n');
    if (line > lines.length) return;
    let start = 0;
    for (let index = 0; index < line - 1; index++) start += lines[index].length + 1;
    node.focus({ preventScroll: true });
    node.setSelectionRange(start, start + lines[line - 1].length);
    const lineHeight = Number.parseFloat(window.getComputedStyle(node).lineHeight) || 20;
    node.scrollTop = Math.max(0, (line - 1) * lineHeight - node.clientHeight / 2);
    positioned.current = target;
  }, [projectId, taskId, path, line, visible, editor.busy, editor.projectId, editor.taskId, editor.path, editor.draft, file]);
  return <article className="dev-review-stack" data-testid="dev-file-source">
    <div className="dev-review-toolbar"><strong className="break-all text-xs">{path}</strong><span role="status" className="text-xs">{t(editor.saving ? 'direct.saving' : dirty ? 'direct.unsaved' : 'direct.saved')}</span></div>
    {invalidLine ? <p role="alert" className="dev-evidence-warning">{t('workbenchFiles.invalidLine')}</p> : null}
    {editor.busy ? <p role="status">{t('review.loading')}</p> : null}
    {editor.error ? <p id={errorId} role="alert" className="dev-feedback dev-feedback-error">{path}: {errorText(editor.error)}</p> : null}
    {editor.error === 'workspace_busy' && taskId ? <ManualTakeover projectId={projectId} taskId={taskId} /> : null}
    {file ? <><details className="dev-technical-details"><summary>{t('agent.details')} · {t('review.bytes', { size: file.size })}</summary><code>{file.hash}</code></details>
      {file.text !== null ? <><label className="dev-review-stack text-xs">{t('direct.source')}<textarea ref={textarea} className="dev-editor" data-testid="dev-file-editor" aria-label={t('direct.source')} aria-invalid={Boolean(editor.error)} aria-describedby={editor.error ? errorId : undefined} spellCheck={false} value={editor.draft} readOnly={!taskId || mutationBlocked} onSelect={event => { const node = event.currentTarget; setSelection(node.selectionStart === node.selectionEnd ? null : { start: node.value.slice(0, node.selectionStart).split('\n').length, end: node.value.slice(0, Math.max(node.selectionStart, node.selectionEnd - 1)).split('\n').length }); }} onChange={event => { editor.edit(event.target.value); setSelection(null); }} onKeyDown={event => { if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's' && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) { event.preventDefault(); if (!event.repeat) void editor.save(); } }} /></label>
      {!taskId ? <p className="dev-evidence-warning">{t('direct.taskRequired')}</p> : null}
      <div className="flex flex-wrap gap-2"><Button size="xs" data-testid="dev-file-save" disabled={!taskId || !dirty || editor.saving || mutationBlocked || editor.error === 'file_changed'} onClick={() => void editor.save()}>{t('direct.save')}</Button><Button size="xs" variant="secondary" disabled={editor.saving || mutationBlocked} onClick={() => void editor.reloadDisk()}>{t('direct.reload')}</Button></div></> : <p className="text-xs">{t(`review.reason.${file.reason ?? 'unavailable'}`)}</p>}
      {onReference ? <div className="dev-review-toolbar"><Button size="xs" variant="secondary" disabled={dirty || editor.busy || editor.saving || !file.hash} onClick={() => attach(false)}>{t('workbenchFiles.referenceFile')}</Button><Button size="xs" variant="secondary" disabled={dirty || editor.busy || editor.saving || !file.hash || !selection} onClick={() => attach(true)}>{t('workbenchFiles.referenceSelection')}</Button>{dirty ? <p className="text-xs">{t('workbenchFiles.saveBeforeReference')}</p> : null}</div> : null}
    </> : null}
    {editor.error === 'file_changed' ? <section className="dev-review-stack" aria-label={t('direct.conflict')} data-testid="dev-file-conflict"><p role="alert" className="dev-evidence-warning">{t('direct.conflict')}</p><div className="dev-comparison"><div><h3 className="text-xs">{t('direct.localDraft')}</h3><pre className="dev-diff" tabIndex={0}>{editor.draft}</pre></div><div><h3 className="text-xs">{t('direct.diskContent')}</h3><pre className="dev-diff" tabIndex={0}>{editor.disk?.text ?? t('direct.diskUnavailable')}</pre></div></div><Button size="xs" variant="secondary" data-testid="dev-file-reload" disabled={editor.saving} onClick={() => void editor.reloadDisk()}>{t('direct.reloadDiscard')}</Button></section> : null}
  </article>;
}

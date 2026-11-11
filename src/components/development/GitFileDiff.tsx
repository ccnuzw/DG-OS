import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { ChevronUp, ChevronDown, Check, Undo2, RefreshCw } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { readDevelopmentDiff, type DevDiff } from '../../stores/developmentReview';
import { readDevelopmentProjectDiff } from '../../stores/developmentExplorer';
import { readDevelopmentGit, readDevelopmentWorkspaces, executeDevelopmentGit, dispatchDeliveryRequest, deliveryRequests, type GitAction } from '../../stores/developmentDelivery';
import { guardDevelopmentDocuments, useDevelopmentEditor, documentId } from '../../stores/developmentEditor';
import type { ConversationReference } from '../../stores/developmentReferences';
import { confirmAction, errorText } from './shared';
import { developmentDraftKey, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { SourceLines } from './ProjectReview';

export type DiffSource = 'worktree' | 'index' | 'task';
export function parseUnifiedDiff(diff: string) {
  let before = 0, after = 0, hunk = -1, beforeRemaining = 0, afterRemaining = 0;
  return diff.split('\n').map(text => {
    const header = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(text);
    if (header) {
      before = Number(header[1]); after = Number(header[3]);
      beforeRemaining = Number(header[2] ?? 1); afterRemaining = Number(header[4] ?? 1);
      ++hunk;
      return { text, before: null, after: null, hunk, kind: 'hunk' };
    }
    if (text.startsWith('diff --git ')) beforeRemaining = afterRemaining = 0;
    if (hunk < 0 || beforeRemaining === 0 && afterRemaining === 0 || text.startsWith('\\') || text === '') return { text, before: null, after: null, hunk, kind: 'meta' };
    if (text.startsWith('+') && afterRemaining > 0) { --afterRemaining; return { text, before: null, after: after++, hunk, kind: 'added' }; }
    if (text.startsWith('-') && beforeRemaining > 0) { --beforeRemaining; return { text, before: before++, after: null, hunk, kind: 'deleted' }; }
    if (text.startsWith(' ') && beforeRemaining > 0 && afterRemaining > 0) { --beforeRemaining; --afterRemaining; return { text, before: before++, after: after++, hunk, kind: 'context' }; }
    return { text, before: null, after: null, hunk, kind: 'meta' };
  });
}
export function GitFileDiff({ projectId, taskId, workspacePath, path, source, ensureTask, onSourceChange, onOpenFile, onReference, onChanged }: { projectId: string; taskId: string | null; workspacePath: string; path: string; source: DiffSource; ensureTask?: () => Promise<string | null>; onSourceChange(source: DiffSource): void; onOpenFile(): void; onReference?: (reference: ConversationReference) => void; onChanged?: () => void }) {
  const { t } = useLocale();
  const [value, setValue] = useState<DevDiff | null>(null), [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false), [acting, setActing] = useState(false);
  const [mode, setMode] = useState('unified'), [selected, setSelected] = useState(0);
  const [reviewed, setReviewed] = useState<string | null>(null);
  const request = useRef(0), root = useRef<HTMLDivElement>(null), actionLock = useRef(false);
  const identity = `${projectId}:${taskId}:${workspacePath}:${path}:${source}`;
  const currentIdentity = useRef(identity); currentIdentity.current = identity;
  const refresh = useCallback(async () => {
    const ticket = ++request.current, epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    setBusy(true); setError(null); setValue(null); setSelected(0);
    const lockKey = developmentDraftKey('delivery', projectId, taskId, workspacePath, 'git-hunk', path);
    const draft = useDevelopmentDrafts.getState().entries[lockKey]?.value as { operation?: { requestId?: string; writeTask?: string } } | undefined;
    if (draft?.operation?.requestId && draft.operation.writeTask) {
      const receipts = await readDevelopmentWorkspaces({ project_id: projectId, task_id: draft.operation.writeTask });
      if (ticket !== request.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
      const receipt = receipts.ok ? receipts.value.operations.find(operation => operation.id === draft.operation?.requestId) : null;
      if (receipt && ['succeeded', 'failed'].includes(receipt.status)) useDevelopmentDrafts.setState(state => { const entries = { ...state.entries }; delete entries[lockKey]; return { entries }; });
    }
    const result = taskId ? await readDevelopmentDiff({ task_id: taskId, path, source }) : await readDevelopmentProjectDiff({ project_id: projectId, path, source: source === 'index' ? 'index' : 'worktree' });
    if (ticket !== request.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    setBusy(false); if (result.ok) setValue(result.value); else setError(result.code);
  }, [projectId, taskId, workspacePath, path, source]);
  useEffect(() => { void refresh(); return () => { ++request.current; }; }, [refresh]);
  const rows = useMemo(() => parseUnifiedDiff(value?.diff ?? ''), [value?.diff]);
  const count = rows.filter(row => row.kind === 'hunk').length;
  const [contentVersion, setContentVersion] = useState<string | null>(null);
  const previousVersion = useRef<string | null>(null);
  useEffect(() => {
    let active = true;
    setContentVersion(null);
    if (value) {
      void crypto.subtle.digest('SHA-256', new TextEncoder().encode(JSON.stringify([identity, value.diff_hash, value.diff, value.before, value.after]))).then(hash => {
        if (active) {
          const version = Array.from(new Uint8Array(hash), byte => byte.toString(16).padStart(2, '0')).join('');
          if (previousVersion.current !== version) setReviewed(null);
          previousVersion.current = version;
          setContentVersion(version);
        }
      });
    }
    return () => { active = false; };
  }, [identity, value]);
  function navigate(index: number) {
    setSelected(index); setMode('unified');
    window.requestAnimationFrame(() => root.current?.querySelector<HTMLElement>(`[data-hunk="${index}"]`)?.scrollIntoView?.({ block: 'center', behavior: 'auto' }));
  }
  async function undo() {
    if ((!taskId && !ensureTask) || !value || value.reason || !value.diff || acting || actionLock.current || source === 'task') return;
    const displayed = value, hunkIndex = selected, epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    const valid = () => epoch === useDevelopment.getState().epoch && generation === contextGeneration() && currentIdentity.current === identity;
    actionLock.current = true; setActing(true); setError(null);
    try {
      if (!await confirmAction(t('fileWorkspace.undo'), t(source === 'index' ? 'fileWorkspace.unstageConfirm' : 'fileWorkspace.undoConfirm', { path, hunk: hunkIndex + 1 }), t('fileWorkspace.undo')) || !valid()) return;
      if (source === 'worktree' && (!await guardDevelopmentDocuments(projectId, taskId, [path], workspacePath) || !valid())) return;
      const writeTask = taskId ?? await ensureTask?.();
      if (!writeTask || !valid()) return;
      const documents = Object.values(useDevelopmentEditor.getState().documents).filter(document => document.projectId === projectId && document.taskId === taskId && document.workspacePath === workspacePath && document.path === path);
      const [review, workspace, latest] = await Promise.all([readDevelopmentGit(writeTask), readDevelopmentWorkspaces({ project_id: projectId, task_id: writeTask }), readDevelopmentDiff({ task_id: writeTask, path, source })]);
      if (!valid()) return;
      if (!review.ok || !workspace.ok || !latest.ok) { setError(!review.ok ? review.code : !workspace.ok ? workspace.code : !latest.ok ? latest.code : 'unknown'); return; }
      if (review.value.path !== workspacePath || latest.value.diff !== displayed.diff || latest.value.diff_hash !== displayed.diff_hash || source === 'worktree' && !displayed.diff_hash) { setError('git_review_stale'); return; }
      const owner = useDevelopment.getState().projects.find(project => project.id === projectId)?.owner;
      if (!owner || deliveryRequests(owner).some(item => item.project_id === projectId && item.task_id === writeTask) || workspace.value.operations.some(operation => operation.status === 'pending' || operation.status === 'unknown')) { setError('workspace_busy'); return; }
      if (source === 'worktree' && documents.some(document => useDevelopmentEditor.getState().documents[document.id] !== document)) { setError('file_changed'); return; }
      const action: GitAction = source === 'index' ? { kind: 'unstage', paths: [path] } : { kind: 'restore_hunk', path, diff_hash: displayed.diff_hash!, hunk_index: hunkIndex, confirmed: true };
      const id = crypto.randomUUID();
      const lockKey = developmentDraftKey('delivery', projectId, taskId, workspacePath, 'git-hunk', path);
      const lock = { action: 'restore_hunk', path, status: 'pending', requestId: id, writeTask };
      if (source === 'worktree') useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [lockKey]: { kind: 'delivery', value: { operation: lock }, baseline: {} } } }));
      const result = await dispatchDeliveryRequest({ id, project_id: projectId, task_id: writeTask, kind: `git_${action.kind}` }, () => executeDevelopmentGit({ id, task_id: writeTask, review_token: review.value.review_token, action }));
      if (source === 'worktree' && epoch === useDevelopment.getState().epoch) {
        useDevelopmentDrafts.setState(state => {
          const entries = { ...state.entries };
          if (result.ok && ['succeeded', 'failed'].includes(result.value.status) || !result.ok && !deliveryRequests(owner).some(request => request.id === id)) delete entries[lockKey];
          else entries[lockKey] = { kind: 'delivery', value: { operation: { ...lock, status: 'unknown', requestId: id, writeTask } }, baseline: {} };
          return { entries };
        });
      }
      if (!valid()) return;
      if (!result.ok) { setError(result.code); return; }
      if (result.value.status !== 'succeeded') { setError(result.value.details.error ?? (result.value.status === 'unknown' ? 'unknown' : 'workspace_busy')); return; }
      if (source === 'worktree') {
        const document = useDevelopmentEditor.getState().documents[documentId(projectId, taskId, path, workspacePath)];
        if (document && documents.includes(document)) await useDevelopmentEditor.getState().open(projectId, taskId, path, workspacePath, true);
      }
      onChanged?.(); await refresh();
    } finally { actionLock.current = false; if (valid()) setActing(false); }
  }
  async function attach() {
    if (!value?.diff || value.reason || !onReference) return;
    const epoch = useDevelopment.getState().epoch, generation = contextGeneration(), displayed = value;
    const hash = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(displayed.diff));
    if (epoch !== useDevelopment.getState().epoch || generation !== contextGeneration() || currentIdentity.current !== identity) return;
    onReference({ project_id: projectId, task_id: taskId, path, kind: 'diff', diff_source: source, expected_hash: Array.from(new Uint8Array(hash), byte => byte.toString(16).padStart(2, '0')).join('') });
  }
  return <article className="git-file-diff dev-review-stack" data-testid="dev-file-diff">
    <div className="file-diff-toolbar"><label className="text-xs">{t('workbenchFiles.diffSource')}<select aria-label={t('workbenchFiles.diffSource')} value={source} disabled={acting} onChange={event => onSourceChange(event.target.value as DiffSource)}>{(taskId ? ['worktree', 'index', 'task'] : ['worktree', 'index']).map(item => <option key={item} value={item}>{t(`workbenchFiles.${item}Diff`)}</option>)}</select></label><Button size="xs" variant="ghost" disabled={busy || acting} aria-label={t('review.refresh')} onClick={() => void refresh()}><RefreshCw size={14} aria-hidden="true" /></Button><Button size="xs" variant="ghost" onClick={onOpenFile}>{t('workbenchFiles.openFile')}</Button>{onReference ? <Button size="xs" variant="ghost" disabled={!value?.diff || Boolean(value.reason) || busy || acting} onClick={() => void attach()}>{t('workbenchFiles.referenceDiff')}</Button> : null}</div>
    <div className="file-diff-toolbar"><Button size="xs" variant="ghost" aria-label={t('fileWorkspace.previousHunk')} disabled={!count || selected === 0} onClick={() => navigate(selected - 1)}><ChevronUp size={14} aria-hidden="true" /></Button><span role="status" className="text-xs">{t('fileWorkspace.hunks', { current: count ? selected + 1 : 0, count })}</span><Button size="xs" variant="ghost" aria-label={t('fileWorkspace.nextHunk')} disabled={!count || selected >= count - 1} onClick={() => navigate(selected + 1)}><ChevronDown size={14} aria-hidden="true" /></Button><Button size="xs" variant="ghost" disabled={!contentVersion || !value?.diff || Boolean(value.reason) || busy || acting} aria-pressed={Boolean(contentVersion && reviewed === contentVersion)} onClick={() => setReviewed(contentVersion)}><Check size={14} aria-hidden="true" />{t(reviewed && reviewed === contentVersion ? 'fileWorkspace.reviewed' : 'fileWorkspace.keep')}</Button>{source !== 'task' ? <Button size="xs" variant="ghost" disabled={(!taskId && !ensureTask) || !count || busy || acting || source === 'worktree' && !value?.diff_hash} onClick={() => void undo()}><Undo2 size={14} aria-hidden="true" />{t(source === 'index' ? 'fileWorkspace.unstageFile' : 'fileWorkspace.undoHunk')}</Button> : null}</div>
    <nav className="file-diff-toolbar" aria-label={t('review.diffView')}>{['unified', 'before', 'after'].map(item => <Button key={item} size="xs" variant={mode === item ? 'secondary' : 'ghost'} aria-pressed={mode === item} onClick={() => setMode(item)}>{t(`review.${item}`)}</Button>)}</nav>
    {busy || acting ? <p role="status">{t('review.loading')}</p> : null}{error ? <p role="alert" className="dev-feedback-error">{errorText(error)}</p> : null}
    {value?.reason ? <p role="status">{t(`review.reason.${value.reason}`)}</p> : null}
    {value && mode !== 'unified' ? <SourceLines text={(mode === 'before' ? value.before : value.after) ?? t('review.reason.unavailable')} label={t(`review.${mode}`)} /> : null}
    {value && mode === 'unified' ? <div ref={root} className="file-unified-diff" tabIndex={0} role="region" aria-label={t('review.unified')}>{rows.map((row, index) => <div key={index} className={`file-diff-line file-diff-${row.kind}`} data-hunk={row.kind === 'hunk' ? row.hunk : undefined} data-selected={row.hunk === selected} onClick={() => { if (row.hunk >= 0) setSelected(row.hunk); }}><span className="file-diff-number" aria-hidden="true">{row.before}</span><span className="file-diff-number" aria-hidden="true">{row.after}</span><code>{row.text || ' '}</code></div>)}</div> : null}
  </article>;
}

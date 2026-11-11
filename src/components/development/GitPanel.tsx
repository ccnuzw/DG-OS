import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Button, ContextMenu, DropdownMenu, Modal, type MenuItem } from '@dgos/ui';
import { ArrowDown, ArrowUp, Check, ChevronDown, File, GitBranch, History, List, MoreHorizontal, Plus, RefreshCw, Sparkles, TreePine, Undo2 } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { deliveryRequests, dispatchDeliveryRequest, executeDevelopmentGit, forgetDeliveryRequests, previewDevelopmentGitPull, readDevelopmentGit, readDevelopmentGitHistory, readDevelopmentStagedDiff, readDevelopmentWorkspaces, reconcileDevelopmentGit, type GitAction, type GitFile, type GitHistoryEntry, type GitHistoryPage, type GitOperation, type GitPullPreview, type GitReview, type WorkspaceView } from '../../stores/developmentDelivery';
import { readDevelopmentGitDetails, type GitDetails } from '../../stores/developmentGitDetails';
import { inspectDevelopmentEntry, mutateDevelopmentEntry, openDevelopmentEntryDirectory, readDevelopmentProjectGit, readDevelopmentProjectGitHistory, type ProjectGitOverview } from '../../stores/developmentExplorer';
import { developmentDraftKey, useDevelopmentDrafts, useScopedDevelopmentDraft } from '../../stores/developmentDrafts';
import { guardDevelopmentDocuments, invalidateDevelopmentDocuments, useDevelopmentEditor } from '../../stores/developmentEditor';
import { confirmAction, errorText } from './shared';
import './git-panel.css';

export interface GitPanelProps {
  projectId: string; taskId: string | null; workspacePath?: string;
  ensureTask?: () => Promise<string | null>;
  sessions: { id: string; engine: 'codex' | 'claude' }[]; live: Record<string, string>;
  refreshKey?: string | number; onOpenFile?: (path: string) => void;
  onOpenDiff?: (path: string, source?: 'index' | 'worktree') => void;
  onChanged?: () => void;
  selectedCommit?: string | null;
  onSelectCommit?: (oid: string, trigger: HTMLButtonElement, workspacePath: string) => void;
  onGenerateMessage?: (stagedDiff: string, signal: AbortSignal) => Promise<string | null>;
}
type Overview = GitReview | ProjectGitOverview;
type ActionStatus = GitOperation['status'] | 'cancelled' | 'stale';
const sameReview = (a: Overview, b: Overview) => a.path === b.path && a.kind === b.kind && a.snapshot_token === b.snapshot_token;
const staged = (file: GitFile) => !!file.index.trim() && file.index !== '?';
const unstaged = (file: GitFile) => !!file.worktree.trim() || file.index === '?';

export function GitPanel(props: GitPanelProps) {
  const epoch = useDevelopment(state => state.epoch);
  return <ScopedGitPanel key={`${epoch}:${props.projectId}:${props.taskId}:${props.workspacePath ?? ''}`} {...props} />;
}
function ScopedGitPanel({ projectId, taskId: initialTaskId, workspacePath, ensureTask, onOpenFile, onOpenDiff, onChanged, onGenerateMessage, refreshKey, selectedCommit, onSelectCommit }: GitPanelProps) {
  const { t, locale } = useLocale();
  const resolvedTask = useRef(initialTaskId), mounted = useRef(true), working = useRef(false), refreshPending = useRef(false), refreshTicket = useRef(0);
  const [review, setReview] = useState<Overview | null>(null), [details, setDetails] = useState<GitDetails | null>(null);
  const reviewRef = useRef(review); reviewRef.current = review;
  const [busy, setBusy] = useState(false), [error, setError] = useState<string | null>(null), [operation, setOperation] = useState<GitOperation | null>(null);
  const [operations, setOperations] = useState<GitOperation[]>([]), [tree, setTree] = useState(true);
  const [network, setNetwork] = useState<'push' | 'pull' | null>(null), [remote, setRemote] = useState(''), [remoteBranch, setRemoteBranch] = useState('');
  const [preview, setPreview] = useState<(GitPullPreview & { kind: 'push' | 'pull' }) | null>(null);
  const [historyPath, setHistoryPath] = useState<string | null>(null), [pathHistory, setPathHistory] = useState<GitHistoryEntry[] | null>(null), [pathTruncated, setPathTruncated] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null), historyTicket = useRef(0);
  const [historyPage, setHistoryPage] = useState<GitHistoryPage | null>(null), [historyLoading, setHistoryLoading] = useState(false), [pageError, setPageError] = useState<string | null>(null), pageTicket = useRef(0);
  const [legacyScopes, setLegacyScopes] = useState<string[]>([]);
  const [generating, setGenerating] = useState(false), generation = useRef(0), controller = useRef<AbortController | null>(null), edits = useRef(0);
  const [generationError, setGenerationError] = useState<string | null>(null);
  const draftScope = workspacePath ?? review?.path ?? 'git-panel';
  const draftKey = developmentDraftKey('commit', projectId, initialTaskId ?? '', draftScope);
  const draft = useScopedDevelopmentDraft('commit', [projectId, initialTaskId ?? '', draftScope], '');
  const draftEntries = useDevelopmentDrafts(state => state.entries);
  const legacyKeys = legacyScopes.map(scope => developmentDraftKey('commit', projectId, initialTaskId ?? '', scope)).filter(key => key !== draftKey);
  const legacyDrafts = legacyKeys.flatMap(key => { const entry = draftEntries[key]; return entry?.kind === 'commit' && typeof entry.value === 'string' && entry.value ? [{ key, text: entry.value }] : []; });
  const scopedLegacyKeys = legacyScopes.filter(scope => scope !== '' && scope !== 'git-panel').map(scope => developmentDraftKey('commit', projectId, initialTaskId ?? '', scope)).filter(key => key !== draftKey);
  useEffect(() => {
    if (!scopedLegacyKeys.length) return;
    useDevelopmentDrafts.setState(state => {
      let entries = state.entries;
      for (const key of scopedLegacyKeys) {
        const saved = entries[key], current = entries[draftKey];
        if (saved?.kind !== 'commit' || typeof saved.value !== 'string' || !saved.value || current && current.value !== '' && current.value !== saved.value) continue;
        if (entries === state.entries) entries = { ...entries };
        entries[draftKey] = current?.value ? current : { kind: 'commit', value: saved.value, baseline: '' };
        delete entries[key];
      }
      return entries === state.entries ? state : { entries };
    });
  }, [draftKey, scopedLegacyKeys.join('\u0000'), draftEntries]);
  const draftRef = useRef(draft); draftRef.current = draft;
  const deleteDraft = useScopedDevelopmentDraft<{ operation: { action: 'delete'; path: string; status: 'pending' | 'unknown' } | null }>('delivery', [projectId, initialTaskId, workspacePath ?? '', 'git-delete'], { operation: null });
  const deleteRef = useRef(deleteDraft); deleteRef.current = deleteDraft;
  const worktreeDraft = useScopedDevelopmentDraft<{ operation: { id: string; action: 'restore' | 'pull' | 'switch' | 'merge'; paths: string[]; status: 'pending' | 'unknown' } | null }>('delivery', [projectId, initialTaskId, workspacePath ?? '', 'git-worktree'], { operation: null });
  const worktreeRef = useRef(worktreeDraft); worktreeRef.current = worktreeDraft;
  useEffect(() => {
    const pending = deleteRef.current.value.operation;
    if (pending?.status === 'pending') deleteRef.current.setValue({ operation: { ...pending, status: 'unknown' } });
  }, []);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++refreshTicket.current; ++historyTicket.current; ++pageTicket.current; ++generation.current; controller.current?.abort(); }; }, []);
  useEffect(() => { ++pageTicket.current; setHistoryPage(null); setHistoryLoading(false); setPageError(null); }, [review?.path, review?.head, refreshKey]);
  useEffect(() => {
    if (!controller.current) return;
    ++generation.current; controller.current.abort(); controller.current = null; setGenerating(false);
  }, [review?.path, review?.snapshot_token]);
  const blocked = operations.some(item => item.status === 'pending' || item.status === 'unknown') || !!deleteDraft.value.operation || !!worktreeDraft.value.operation;
  function guard() { const epoch = useDevelopment.getState().epoch, context = contextGeneration(); return () => mounted.current && epoch === useDevelopment.getState().epoch && context === contextGeneration(); }
  function receipts(workspace: WorkspaceView, task: string | null) {
    const owner = useDevelopment.getState().projects.find(item => item.id === projectId)?.owner;
    const rows = [...workspace.operations];
    if (owner) {
      forgetDeliveryRequests(owner, rows);
      const known = new Set(rows.map(item => item.id));
      rows.push(...deliveryRequests(owner).filter(item => item.project_id === projectId && item.task_id === task && !known.has(item.id)).map(item => ({ ...item, status: 'unknown' as const, workspace_id: null, request_fingerprint: '', details: {} })));
    }
    return rows;
  }
  const refresh = useCallback(async () => {
    if (working.current) { refreshPending.current = true; return; }
    const valid = guard(), ticket = ++refreshTicket.current;
    working.current = true; setBusy(true); setError(null);
    try {
      const task = resolvedTask.current;
      ++pageTicket.current; setHistoryPage(null); setHistoryLoading(false); setPageError(null);
      const [g, d, w] = await Promise.all([readDevelopmentProjectGit(projectId, task), readDevelopmentGitDetails({ project_id: projectId, task_id: task }), readDevelopmentWorkspaces({ project_id: projectId, task_id: task })]);
      if (!valid() || ticket !== refreshTicket.current) return;
      if (g.ok) { setReview(g.value); setPreview(null); } else setReview(null);
      setDetails(d.ok ? d.value : null);
      if (w.ok) {
        const rows = receipts(w.value, task); setOperations(rows);
        if (g.ok && (!workspacePath || workspacePath === g.value.path)) {
          const active = w.value.workspaces.find(item => w.value.selected_workspace_id ? item.id === w.value.selected_workspace_id : item.kind === 'original');
          setLegacyScopes([...new Set([...(active?.path === g.value.path ? [active.id] : []), g.value.path, 'git-panel', ''])]);
        } else setLegacyScopes([]);
        const lock = worktreeRef.current.value.operation;
        if (lock) {
          const receipt = rows.find(item => item.id === lock.id);
          if (!receipt || receipt.status === 'succeeded' || receipt.status === 'failed') {
            if (receipt?.status === 'succeeded') invalidateDevelopmentDocuments(projectId, initialTaskId, lock.paths, g.ok ? g.value.path : workspacePath);
            worktreeRef.current.clear();
          } else worktreeRef.current.setValue({ operation: { ...lock, status: receipt.status } });
        }
        setOperation(current => rows.find(item => item.status === 'pending' || item.status === 'unknown') ?? rows.find(item => item.id === current?.id) ?? current);
      }
      setError(!g.ok ? g.code : !d.ok ? d.code : !w.ok ? w.code : null);
    } catch { if (valid()) setError('invalid_request'); }
    finally { working.current = false; if (valid()) setBusy(false); }
  }, [projectId]);
  useEffect(() => { void refresh(); }, [refresh, refreshKey]);
  useEffect(() => { if (!busy && refreshPending.current) { refreshPending.current = false; void refresh(); } }, [busy, refresh]);
  function release() { working.current = false; if (mounted.current) setBusy(false); }

  // Every write reloads both review and ledger. Project browsing never creates a task.
  async function authority(expected: Overview): Promise<GitReview | null> {
    const valid = guard();
    const task = resolvedTask.current ?? await ensureTask?.() ?? null;
    if (!task || !valid()) return null;
    const [g, w] = await Promise.all([readDevelopmentGit(task), readDevelopmentWorkspaces({ project_id: projectId, task_id: task })]);
    if (!valid()) return null;
    if (!g.ok || !w.ok) { setError(!g.ok ? g.code : !w.ok ? w.code : 'unknown'); return null; }
    const rows = receipts(w.value, task); setOperations(rows);
    if (rows.some(item => item.status === 'unknown' || item.status === 'pending')) { setOperation(rows.find(item => item.status === 'unknown' || item.status === 'pending')!); return null; }
    if (!sameReview(expected, g.value) || workspacePath && workspacePath !== g.value.path) { setReview(g.value); setPreview(null); setError('git_review_stale'); return null; }
    resolvedTask.current = task; setReview(g.value); return g.value;
  }
  async function act(action: GitAction, expected: Overview | null = review): Promise<ActionStatus> {
    if (!expected || working.current || blocked) return 'cancelled';
    const valid = guard(); working.current = true; setBusy(true); setError(null);
    try {
      const worktreeAction = action.kind === 'restore' || action.kind === 'pull' || action.kind === 'switch' || action.kind === 'merge';
      const paths = action.kind === 'restore' ? action.paths : [''];
      // Saving an editor draft can change the repository snapshot. Review only
      // afterwards, and reject changed snapshots rather than replacing consent.
      if (worktreeAction && !await guardDevelopmentDocuments(projectId, initialTaskId, paths, expected.path)) return 'cancelled';
      if (!valid()) return 'cancelled';
      const documents = worktreeAction ? Object.values(useDevelopmentEditor.getState().documents).filter(document => document.projectId === projectId && document.taskId === initialTaskId && document.workspacePath === expected.path && paths.some(path => path === '' || document.path === path || document.path.startsWith(`${path}/`))) : [];
      const current = await authority(expected);
      if (!current || !valid()) return 'stale';
      // The explicit Pull click obtains a host preview and checks it against the confirmed target.
      if (action.kind === 'pull') {
        const target = await previewDevelopmentGitPull(current.task_id, action.remote, action.branch);
        if (!valid()) return 'cancelled';
        if (!target.ok) { setError(target.code); return 'stale'; }
        if (!preview || preview.kind !== 'pull' || preview.remote_display !== target.value.remote_display || preview.remote_ref !== target.value.remote_ref || target.value.review_token !== current.review_token) { setPreview(null); setError('git_pull_preview_stale'); return 'stale'; }
        action = { ...action, preview_token: target.value.preview_token };
      }
      if (action.kind === 'push') {
        const target = await readDevelopmentGitDetails({ project_id: projectId, task_id: current.task_id });
        if (!valid()) return 'cancelled';
        if (!target.ok) { setError(target.code); return 'stale'; }
        if (!preview || preview.kind !== 'push' || preview.remote !== action.remote || target.value.remote_push_targets[action.remote]?.join('\n') !== preview.remote_display) { setPreview(null); setError('git_review_stale'); return 'stale'; }
      }
      if (worktreeAction && (documents.some(document => useDevelopmentEditor.getState().documents[document.id] !== document) || Object.values(useDevelopmentEditor.getState().documents).filter(document => document.projectId === projectId && document.taskId === initialTaskId && document.workspacePath === expected.path && paths.some(path => path === '' || document.path === path || document.path.startsWith(`${path}/`))).length !== documents.length)) return 'cancelled';
      const id = crypto.randomUUID(), identity = { id, project_id: projectId, task_id: current.task_id, kind: `git_${action.kind}` };
      const pending: GitOperation = { ...identity, status: 'pending', workspace_id: null, request_fingerprint: '', details: {} };
      setOperation(pending); setOperations(rows => [...rows, pending]);
      if (action.kind === 'restore' || action.kind === 'pull' || action.kind === 'switch' || action.kind === 'merge') worktreeRef.current.setValue({ operation: { id, action: action.kind, paths: action.kind === 'restore' ? action.paths : [''], status: 'pending' } });
      const result = await dispatchDeliveryRequest(identity, () => executeDevelopmentGit({ id, task_id: current.task_id, review_token: current.review_token, action }));
      if (worktreeAction) {
        if (result.ok && (result.value.status === 'succeeded' || result.value.status === 'failed')) worktreeRef.current.clear();
        else { const lock = worktreeRef.current.value.operation; if (lock?.id === id) worktreeRef.current.setValue({ operation: { ...lock, status: 'unknown' } }); }
      }
      if (action.kind === 'commit' && result.ok && result.value.status === 'succeeded') draftRef.current.clear(action.message);
      if (!valid()) return result.ok ? result.value.status : 'unknown';
      if (result.ok) {
        setOperation(result.value);
        if (result.value.details.error) setError(result.value.details.error);
        if (result.value.status === 'succeeded') {
          if (action.kind === 'restore' || action.kind === 'switch' || action.kind === 'pull' || action.kind === 'merge') invalidateDevelopmentDocuments(projectId, initialTaskId, action.kind === 'restore' ? action.paths : [''], expected.path);
          onChanged?.();
        }
      } else setError(result.code);
      const failure = !result.ok ? result.code : result.value.details.error;
      release(); await refresh(); if (valid() && failure) setError(failure);
      return result.ok ? result.value.status : 'unknown';
    } catch { if (valid()) setError('invalid_request'); return 'unknown'; }
    finally { release(); }
  }
  async function restore(file: GitFile) {
    if (!review || working.current || blocked) return;
    const valid = guard(), expected = review;
    const warning = file.index === '?' ? t('gitPanel.restoreUntracked') : t('gitPanel.restoreWarning');
    if (!await confirmAction(t('gitPanel.restore'), `${warning}\n${file.path}`, t('gitPanel.restore')) || !valid()) return;
    await act({ kind: 'restore', paths: [file.path], confirmed: true }, expected);
  }
  async function commit(push = false) {
    if (!review || !draft.value.trim() || working.current || blocked) return;
    const valid = guard(), expected = review, message = draft.value;
    const paths = review.files.filter(staged).map(file => file.path);
    const preexisting = 'initial_staged_paths' in review ? review.initial_staged_paths : [];
    const content = `${t('gitPanel.commitWarning')}\n${paths.join('\n') || t('gitPanel.mergeCommit')}\n${preexisting.length ? `${t('gitPanel.preexisting')}\n${preexisting.join('\n')}` : ''}`;
    if (!await confirmAction(t('delivery.commit'), content, t('delivery.commit')) || !valid() || draftRef.current.value !== message) return;
    const result = await act({ kind: 'commit', message, confirm_full_index: true, confirm_preexisting_staged: true }, expected);
    if (result === 'succeeded' && push && valid()) { setPreview(null); setRemote(''); setRemoteBranch(''); setNetwork('push'); }
  }
  async function generate() {
    if (!onGenerateMessage || !review?.files.some(staged) || generating || busy || blocked) return;
    const valid = guard(), expected = review, ticket = ++generation.current, before = edits.current, message = draftRef.current.value;
    const abort = new AbortController(); controller.current = abort; setGenerating(true); setGenerationError(null);
    const active = () => valid() && ticket === generation.current && !abort.signal.aborted;
    try {
      const preview = await readDevelopmentStagedDiff({ project_id: projectId, task_id: resolvedTask.current });
      if (!active()) return;
      if (!preview.ok) { setGenerationError(errorText(preview.code)); return; }
      if (preview.value.snapshot_token !== expected.snapshot_token || !reviewRef.current || !sameReview(expected, reviewRef.current)) { setGenerationError(errorText('git_review_stale')); return; }
      if (before !== edits.current || message !== draftRef.current.value) return;
      const text = await onGenerateMessage(preview.value.diff, abort.signal);
      if (active() && before === edits.current && message === draftRef.current.value && text !== null) draftRef.current.setValue(text);
    } catch (failure) { if (active()) setGenerationError(failure instanceof Error ? failure.message : t('gitPanel.generation_failed')); }
    finally { if (controller.current === abort) controller.current = null; if (valid() && ticket === generation.current) setGenerating(false); }
  }
  async function prepareNetwork() {
    if (!review || !remote || !remoteBranch || working.current || blocked) return;
    const valid = guard(), expected = review; working.current = true; setBusy(true); setError(null);
    try {
      const target = await readDevelopmentGitDetails({ project_id: projectId, task_id: resolvedTask.current });
      if (!valid()) return;
      if (!target.ok) { setError(target.code); return; }
      const urls = network === 'push' ? target.value.remote_push_targets[remote] : target.value.remote_fetch_targets[remote];
      if (!urls || urls.length !== 1) { setError('unsafe_git_remote'); return; }
      setPreview({ kind: network === 'push' ? 'push' : 'pull', review_token: 'review_token' in expected ? expected.review_token : '', remote, branch: remoteBranch, remote_display: urls.join('\n'), remote_ref: `refs/heads/${remoteBranch}`, preview_token: '' });
    } finally { release(); }
  }
  async function sendNetwork() {
    if (!review || !preview || !network) return;
    const valid = guard(), expected = review, value = preview, kind = network;
    const warning = t(kind === 'pull' ? 'delivery.pullTrust' : 'delivery.pushTrust');
    if (!await confirmAction(t(`delivery.${kind}`), `${warning}\n${review.path}\n${preview.remote_display}\n${preview.remote_ref}\n${review.branch ?? ''}`, t(`delivery.${kind}`)) || !valid()) return;
    const result = await act(kind === 'pull' ? { kind, remote: value.remote, branch: value.branch, preview_token: value.preview_token, trust_directory: true, trust_network: true } : { kind, remote: value.remote, trust_directory: true, trust_network: true }, expected);
    if (valid()) { setPreview(null); if (result === 'succeeded') setNetwork(null); }
  }
  async function reconcile(item: GitOperation) {
    if (working.current) return;
    const valid = guard(), push = item.kind === 'git_push';
    if (push && !await confirmAction(t('delivery.reconcile'), t('delivery.pushTrust'), t('delivery.reconcile')) || !valid()) return;
    working.current = true; setBusy(true);
    const result = await reconcileDevelopmentGit({ id: item.id, trust_directory: push, trust_network: push });
    await useDevelopment.getState().load();
    release(); if (!valid()) return;
    if (result.ok) setOperation(result.value);
    await refresh(); if (!result.ok && valid()) setError(result.code);
  }
  async function history(path: string) {
    const valid = guard(), ticket = ++historyTicket.current;
    setHistoryPath(path); setPathHistory(null); setHistoryError(null);
    const result = await readDevelopmentGitDetails({ project_id: projectId, task_id: resolvedTask.current, path });
    if (!valid() || ticket !== historyTicket.current) return;
    if (result.ok) { setPathHistory(result.value.history); setPathTruncated(result.value.history_truncated); } else setHistoryError(result.code);
  }
  async function moreHistory() {
    if (!review?.head || historyLoading || historyPage && !historyPage.next_cursor) return;
    const valid = guard(), ticket = ++pageTicket.current, expected = review;
    const input = { start_oid: historyPage?.anchor ?? expected.head, cursor: historyPage?.next_cursor ?? null, limit: 50 };
    setHistoryLoading(true); setPageError(null);
    const task = initialTaskId;
    const result = task ? await readDevelopmentGitHistory({ ...input, task_id: task }) : await readDevelopmentProjectGitHistory({ ...input, project_id: projectId });
    if (!valid() || ticket !== pageTicket.current || reviewRef.current?.path !== expected.path || reviewRef.current.head !== expected.head) return;
    if (result.ok) setHistoryPage(previous => previous ? { ...result.value, commits: [...previous.commits, ...result.value.commits.filter(entry => !previous.commits.some(old => old.oid === entry.oid))] } : result.value);
    else setPageError(result.code);
    setHistoryLoading(false);
  }
  async function reveal(path: string) {
    const valid = guard(); const result = await openDevelopmentEntryDirectory({ project_id: projectId, task_id: resolvedTask.current, path });
    if (valid() && !result.ok) setError(result.code);
  }
  async function copyPath(path: string) {
    if (!review) return; const valid = guard();
    try { await navigator.clipboard.writeText(`${review.path.replace(/[\\/]$/, '')}/${path}`); }
    catch { if (valid()) setError('clipboard_failed'); }
  }
  async function remove(file: GitFile) {
    if (!review || working.current || blocked) return;
    const expected = review, valid = guard(); working.current = true; setBusy(true); setError(null);
    try {
      if (!await guardDevelopmentDocuments(projectId, initialTaskId, [file.path], expected.path) || !valid()) return;
      const current = await authority(expected); if (!current || !valid()) return;
      const input = { project_id: projectId, task_id: current.task_id, path: file.path };
      const inspected = await inspectDevelopmentEntry(input);
      if (!valid()) return;
      if (!inspected.ok) { setError(inspected.code); return; }
      if (!await confirmAction(t('gitPanel.remove'), `${t('gitPanel.removeWarning')}\n${expected.path}/${file.path}`, t('gitPanel.remove')) || !valid()) return;
      // Confirmation may have stayed open while another process changed Git state.
      if (!await guardDevelopmentDocuments(projectId, initialTaskId, [file.path], expected.path) || !valid() || !await authority(expected) || !valid()) return;
      const pending = { operation: { action: 'delete' as const, path: file.path, status: 'pending' as const } };
      deleteRef.current.setValue(pending);
      const result = await mutateDevelopmentEntry({ ...input, expected_version: inspected.value.version, action: 'delete', destination: null, trust_directory: true });
      const unknown = !result.ok && ['file_mutation_unknown', 'unknown'].includes(result.code);
      deleteRef.current.setValue({ operation: unknown || result.ok && !valid() ? { ...pending.operation, status: 'unknown' } : null });
      if (!valid()) return;
      if (result.ok) { invalidateDevelopmentDocuments(projectId, initialTaskId, [file.path], expected.path); onChanged?.(); }
      else setError(result.code);
      release(); await refresh(); if (!result.ok && valid()) setError(result.code);
    } finally { release(); }
  }
  async function inspectDelete() {
    const pending = deleteRef.current.value.operation;
    if (!pending || working.current || pending.status === 'pending') return;
    const valid = guard(); working.current = true; setBusy(true);
    try {
      const result = await inspectDevelopmentEntry({ project_id: projectId, task_id: resolvedTask.current, path: pending.path });
      if (!valid()) return;
      if (!result.ok && result.code !== 'file_not_found') { setError(result.code); return; }
      if (!await confirmAction(t('workbenchFiles.inspectResult'), `${pending.path}: ${t(result.ok ? 'workbenchFiles.present' : 'workbenchFiles.absent')}\n${t('workbenchFiles.acceptObservedDetail')}`, t('workbenchFiles.acceptObserved')) || !valid()) return;
      if (!await guardDevelopmentDocuments(projectId, initialTaskId, [pending.path], review?.path, true) || !valid()) return;
      invalidateDevelopmentDocuments(projectId, initialTaskId, [pending.path], review?.path);
      deleteRef.current.clear(); onChanged?.(); release(); await refresh();
    } finally { release(); }
  }
  function menu(file: GitFile, source: 'index' | 'worktree'): MenuItem[] {
    const disabled = busy || blocked;
    return [
      { kind: 'item', id: 'open-diff', label: t('gitPanel.openDiff'), icon: 'diff', disabled: !onOpenDiff, onSelect: () => onOpenDiff?.(file.path, source) },
      { kind: 'item', id: 'open-file', label: t('gitPanel.openFile'), icon: 'file', disabled: !onOpenFile, onSelect: () => onOpenFile?.(file.path) },
      { kind: 'item', id: 'reveal', label: t('workbenchFiles.openDirectory'), icon: 'folder-open', onSelect: () => void reveal(file.path) },
      { kind: 'item', id: 'copy-path', label: t('gitPanel.copyPath'), icon: 'copy', onSelect: () => void copyPath(file.path) },
      { kind: 'separator' },
      { kind: 'item', id: 'stage', label: t(source === 'index' ? 'delivery.unstage' : 'delivery.stage'), icon: source === 'index' ? 'minus' : 'plus', disabled, onSelect: () => void act({ kind: source === 'index' ? 'unstage' : 'stage', paths: [file.path] }) },
      { kind: 'item', id: 'untrack', label: t(file.index === '?' ? 'gitPanel.untrackUnavailable' : 'gitPanel.untrack'), icon: 'git-branch', disabled: disabled || file.index === '?', onSelect: async () => { const valid = guard(), expected = review; if (await confirmAction(t('gitPanel.untrack'), `${t('gitPanel.untrackWarning')}\n${file.path}`, t('gitPanel.untrack')) && valid()) void act({ kind: 'untrack', paths: [file.path] }, expected); } },
      { kind: 'item', id: 'history', label: t('gitPanel.pathHistory'), icon: 'history', onSelect: () => void history(file.path) },
      { kind: 'separator' },
      { kind: 'item', id: 'restore', label: t('gitPanel.restore'), icon: 'undo-2', disabled: disabled || !unstaged(file), danger: true, onSelect: () => void restore(file) },
      { kind: 'item', id: 'remove', label: t('gitPanel.remove'), icon: 'trash-2', disabled, danger: true, onSelect: () => void remove(file) },
    ];
  }
  const conflicts = new Set(review?.conflicts.map(file => file.path));
  const groups = [
    { id: 'staged', label: 'gitPanel.staged', source: 'index' as const, files: review?.files.filter(file => !conflicts.has(file.path) && staged(file)) ?? [] },
    { id: 'unstaged', label: 'gitPanel.unstaged', source: 'worktree' as const, files: review?.files.filter(file => !conflicts.has(file.path) && unstaged(file)) ?? [] },
    { id: 'conflicts', label: 'delivery.conflicts', source: 'worktree' as const, files: review?.files.filter(file => conflicts.has(file.path)) ?? [] },
  ];
  const totals = details?.files.reduce((sum, file) => ({ added: sum.added + (file.index_added ?? 0) + (file.worktree_added ?? 0), deleted: sum.deleted + (file.index_deleted ?? 0) + (file.worktree_deleted ?? 0) }), { added: 0, deleted: 0 });
  const git = review?.kind === 'git', disabled = busy || blocked;
  const networkBranches = network === 'push' ? (review?.branch ? [review.branch] : []) : details?.remote_branches[remote] ?? [];
  const hasStaged = !!review?.files.some(staged), canCommit = hasStaged || !!review?.merge_heads.length;
  function iconButton(label: string, icon: ReactNode, action: () => void, unavailable = false) {
    return <button type="button" className="git-icon" title={label} aria-label={label} disabled={unavailable} onClick={event => { event.preventDefault(); event.stopPropagation(); action(); }}>{icon}</button>;
  }
  function fileRow(file: GitFile, source: 'index' | 'worktree', prefix: string) {
    const stat = details?.files.find(item => item.path === file.path);
    const added = source === 'index' ? stat?.index_added : stat?.worktree_added;
    const deleted = source === 'index' ? stat?.index_deleted : stat?.worktree_deleted;
    const status = source === 'index' ? file.index : file.index === '?' ? '?' : file.worktree;
    return <ContextMenu key={file.path} items={menu(file, source)}><div className="git-file" data-path={file.path}>
      <button type="button" className="git-file-open" onClick={() => onOpenDiff?.(file.path, source)} title={file.path} disabled={!onOpenDiff}><File size={14} aria-hidden="true" /><span>{tree ? file.path.slice(prefix.length) : file.path.split('/').pop()}{!tree && file.path.includes('/') ? <small>{file.path.slice(0, file.path.lastIndexOf('/'))}</small> : null}</span></button>
      <span className="git-line-stats" aria-label={t('gitPanel.lineStats', { added: added ?? '?', deleted: deleted ?? '?' })}><span className="git-added">{added === null || added === undefined ? '—' : `+${added}`}</span><span className="git-deleted">{deleted === null || deleted === undefined ? '—' : `−${deleted}`}</span></span>
      <span className={`git-status git-status-${status.trim() || 'none'}`} title={t('gitPanel.status', { status })}>{status}</span>
      <div className="git-file-actions">{iconButton(t(source === 'index' ? 'delivery.unstage' : 'delivery.stage'), source === 'index' ? <Undo2 size={13} /> : <Plus size={14} />, () => void act({ kind: source === 'index' ? 'unstage' : 'stage', paths: [file.path] }), disabled)}{source === 'worktree' ? iconButton(t('gitPanel.restore'), <Undo2 size={13} />, () => void restore(file), disabled) : null}</div>
      <DropdownMenu items={menu(file, source)} trigger={<button type="button" className="git-icon git-more" aria-label={t('gitPanel.fileActions', { path: file.path })}><MoreHorizontal size={14} /></button>} />
    </div></ContextMenu>;
  }
  function directory(files: GitFile[], prefix: string, source: 'index' | 'worktree'): ReactNode {
    const dirs = [...new Set(files.map(file => file.path.slice(prefix.length)).filter(path => path.includes('/')).map(path => path.split('/')[0]))].sort();
    return <div className="git-directory">{dirs.map(name => {
      const path = `${prefix}${name}/`, children = files.filter(file => file.path.startsWith(path));
      return <details key={path} open><summary><span>{name}</span><span className="git-count">{children.length}</span></summary>{directory(children, path, source)}</details>;
    })}{files.filter(file => !file.path.slice(prefix.length).includes('/')).map(file => fileRow(file, source, prefix))}</div>;
  }
  function historyList(entries: GitHistoryEntry[], truncated: boolean, fromPath = false) {
    const relative = new Intl.RelativeTimeFormat(locale, { numeric: 'auto', style: 'narrow' });
    function age(timestamp: number) {
      const seconds = Math.round(timestamp - Date.now() / 1000), absolute = Math.abs(seconds);
      const [divisor, unit] = absolute >= 31536000 ? [31536000, 'year'] as const : absolute >= 2592000 ? [2592000, 'month'] as const : absolute >= 86400 ? [86400, 'day'] as const : absolute >= 3600 ? [3600, 'hour'] as const : absolute >= 60 ? [60, 'minute'] as const : [1, 'second'] as const;
      return relative.format(Math.round(seconds / divisor), unit);
    }
    return <><ol className="git-history-list">{entries.map(entry => <li key={entry.oid}><button type="button" className="git-history-row" data-commit-oid={entry.oid} data-selected={selectedCommit === entry.oid} aria-pressed={selectedCommit === entry.oid} disabled={!onSelectCommit} title={`${entry.subject}\n${entry.author}\n${new Date(entry.timestamp * 1000).toLocaleString(locale)}\n${entry.oid}`} onClick={event => {
      if (fromPath) { ++historyTicket.current; setHistoryPath(null); }
      onSelectCommit?.(entry.oid, event.currentTarget, review?.path ?? workspacePath ?? '');
    }}><code>{entry.oid.slice(0, 7)}</code><span>{entry.subject || t('gitPanel.emptyMessage')}</span><time dateTime={new Date(entry.timestamp * 1000).toISOString()} title={new Date(entry.timestamp * 1000).toLocaleString(locale)}>{age(entry.timestamp)}</time></button></li>)}</ol>{!entries.length ? <p className="git-feedback">{t('delivery.noCommits')}</p> : null}{truncated && fromPath ? <p className="git-feedback">{t('gitPanel.pathHistoryBound')}</p> : null}</>;
  }
  return <div className="git-panel" data-testid="dev-git-panel" aria-label={t('workbenchGit.title')}>
    <header className="git-toolbar">
      <GitBranch size={15} aria-hidden="true" />
      <select aria-label={t('gitPanel.branch')} value={review?.branch ?? ''} disabled={!git || disabled || !details} onChange={event => { if (review && event.target.value !== review.branch) void act({ kind: 'switch', branch: event.target.value }); }}>
        {!review?.branch ? <option value="">{t(review?.head ? 'delivery.detached' : 'gitPanel.noHead')}</option> : !details?.branches.includes(review.branch) ? <option value={review.branch}>{review.branch}</option> : null}
        {details?.branches.map(branch => <option key={branch} value={branch}>{branch}</option>)}
      </select>
      <DropdownMenu items={(details?.branches ?? []).filter(branch => branch !== review?.branch).map(branch => ({ kind: 'item' as const, id: `merge-${branch}`, label: t('gitPanel.mergeBranch', { branch }), disabled: !git || disabled || !review?.branch || !review.head || !!review.files.length || !!review.conflicts.length || !!review.merge_heads.length, onSelect: () => void act({ kind: 'merge', branch }) }))} trigger={<button type="button" className="git-icon" aria-label={t('gitPanel.mergeBranches')} title={t('gitPanel.mergeBranches')} disabled={!git || disabled || !details?.branches.some(branch => branch !== review?.branch)}><MoreHorizontal size={14} aria-hidden="true" /></button>} />
      <span className="git-line-stats"><span className="git-added">{totals ? `+${totals.added}` : '—'}</span><span className="git-deleted">{totals ? `−${totals.deleted}` : '—'}</span></span>
      {iconButton(t('delivery.pull'), <ArrowDown size={15} />, () => { setNetwork('pull'); setRemote(''); setRemoteBranch(''); setPreview(null); }, !git || disabled || !details?.remotes.length || !review?.head)}
      {iconButton(t('delivery.push'), <ArrowUp size={15} />, () => { setNetwork('push'); setRemote(''); setRemoteBranch(''); setPreview(null); }, !git || disabled || !details?.remotes.length || !review?.branch || !review.head)}
      {iconButton(t(tree ? 'gitPanel.listView' : 'gitPanel.treeView'), tree ? <List size={15} /> : <TreePine size={15} />, () => setTree(value => !value))}
      {iconButton(t('review.refresh'), <RefreshCw size={14} className={busy ? 'git-refreshing' : ''} />, () => void refresh(), busy)}
    </header>
    {busy ? <p className="git-feedback" role="status">{t('review.loading')}</p> : null}
    {error ? <p className="git-feedback git-error" role="alert">{['generation_failed', 'clipboard_failed'].includes(error) ? t(`gitPanel.${error}`) : errorText(error)}</p> : null}
    {generationError ? <p className="git-feedback git-error" role="alert">{generationError}</p> : null}
    {review && !git ? <p className="git-feedback">{t('delivery.plainDirectory')}</p> : null}
    {git && !review.head ? <p className="git-feedback">{t('gitPanel.noHeadHint')}</p> : null}
    {git && details && !details.remotes.length ? <p className="git-feedback">{t('gitPanel.noRemote')}</p> : null}
    <div className="git-changes">{git ? <>{!review.files.length ? <p className="git-feedback">{t('workbenchGit.noChanges')}</p> : null}{groups.filter(group => group.files.length).map(group => <details className="git-group" key={group.id} open>
      <summary><span>{t(group.label)}</span><span className="git-count">{group.files.length}</span><span className="git-group-action">{iconButton(t(group.source === 'index' ? 'gitPanel.unstageAll' : 'gitPanel.stageAll'), group.source === 'index' ? <Undo2 size={13} /> : <Plus size={14} />, () => void act({ kind: group.source === 'index' ? 'unstage' : 'stage', paths: group.files.map(file => file.path) }), disabled)}</span></summary>
      {tree ? directory(group.files, '', group.source) : group.files.map(file => fileRow(file, group.source, ''))}
    </details>)}</> : null}
    {operation ? <details className="git-operation"><summary>{t(`delivery.operation.${operation.status}`)}</summary><code>{operation.id}</code>{operation.details.error ? <p>{errorText(operation.details.error)}</p> : null}</details> : null}
    {operations.filter(item => item.status === 'pending' || item.status === 'unknown').map(item => <div key={item.id} className="git-feedback"><p>{t('delivery.unknownOperation')}</p><Button size="xs" disabled={busy} onClick={() => void reconcile(item)}>{t('delivery.reconcile')} · {item.id.slice(0, 8)}</Button></div>)}
    {deleteDraft.value.operation ? <div className="git-feedback" role="status">{t('gitPanel.deleteUnknown')} <code>{deleteDraft.value.operation.path}</code><Button size="xs" disabled={busy || deleteDraft.value.operation.status === 'pending'} onClick={() => void inspectDelete()}>{t('workbenchFiles.inspectResult')}</Button></div> : null}
    </div>
    <div className="git-bottom">
      <details className="git-history" open><summary><History size={14} aria-hidden="true" />{t('delivery.gitHistory')}<span className="git-count">{historyPage ? `${historyPage.commits.length}${historyPage.next_cursor ? '+' : ''}` : details ? `${details.history.length}${details.history_truncated ? '+' : ''}` : '—'}</span></summary>{historyPage ? historyList(historyPage.commits, false) : details ? historyList(details.history, details.history_truncated) : <p className="git-feedback">{t('delivery.unknown')}</p>}{pageError ? <p className="git-feedback git-error" role="alert">{errorText(pageError)}</p> : null}{historyLoading ? <p className="git-feedback" role="status">{t('review.loading')}</p> : null}{review?.head && (historyPage ? historyPage.next_cursor : details?.history_truncated) ? <Button size="xs" variant="secondary" disabled={historyLoading || busy} onClick={() => void moreHistory()}>{t(historyPage ? 'delivery.moreGitHistory' : 'delivery.openGitHistory')}</Button> : null}</details>
      <section className="git-commit"><label htmlFor={`git-message-${projectId}`}>{t('delivery.commitMessage')}</label><textarea id={`git-message-${projectId}`} value={draft.value} placeholder={t('gitPanel.messagePlaceholder')} onChange={event => { ++edits.current; draft.setValue(event.target.value); }} rows={3} />
        {legacyDrafts.length ? <section className="git-feedback" aria-label={t('gitPanel.savedDrafts')}><p>{t('gitPanel.draftConflict')}</p>{legacyDrafts.map((saved, index) => <div key={saved.key}><label>{t('gitPanel.savedDraft', { index: index + 1 })}<textarea readOnly value={saved.text} rows={3} /></label><Button size="xs" variant="secondary" onClick={async () => { try { await navigator.clipboard.writeText(saved.text); } catch { setError('clipboard_failed'); } }}>{t('gitPanel.copyDraft')}</Button></div>)}</section> : null}
        <div className="git-commit-tools">{generating ? <button type="button" className="git-generate" onClick={() => { ++generation.current; controller.current?.abort(); controller.current = null; setGenerating(false); }}>{t('gitPanel.cancelGeneration')}</button> : <button type="button" className="git-generate" disabled={!onGenerateMessage || !hasStaged || busy || blocked} title={t(onGenerateMessage ? 'gitPanel.generateHint' : 'gitPanel.generationUnavailable')} onClick={() => void generate()}><Sparkles size={13} aria-hidden="true" />{t('gitPanel.generate')}</button>}{draft.dirty ? <button type="button" className="git-draft-discard" onClick={() => void draft.discard()}>{t('dev.discardDraft')}</button> : null}</div>
        <div className="git-commit-buttons"><button type="button" className="git-commit-primary" disabled={!git || disabled || !draft.value.trim() || !canCommit || !!review?.conflicts.length} onClick={() => void commit()}><Check size={14} aria-hidden="true" />{t('delivery.commit')}</button><DropdownMenu items={[{ kind: 'item', id: 'commit-push', label: t('gitPanel.commitPush'), disabled: !git || disabled || !draft.value.trim() || !canCommit || !!review?.conflicts.length || !details?.remotes.length || !review?.branch, onSelect: () => void commit(true) }]} trigger={<button type="button" className="git-commit-menu" aria-label={t('gitPanel.commitActions')} disabled={disabled}><ChevronDown size={14} /></button>} /></div>
      </section>
      <footer className="git-statusbar"><GitBranch size={12} aria-hidden="true" /><span title={review?.path}>{review?.branch ?? t('delivery.unknown')}</span><span title={details?.upstream ?? t('gitPanel.noUpstream')}>↑{details?.ahead ?? '—'} ↓{details?.behind ?? '—'}</span><span className="git-status-counts">{t('gitPanel.statusCounts', { modified: review?.files.filter(file => file.index !== '?' && (staged(file) || unstaged(file))).length ?? '—', untracked: review?.files.filter(file => file.index === '?').length ?? '—' })}</span></footer>
    </div>
    <Modal open={!!network} onOpenChange={open => { if (!open) { setNetwork(null); setPreview(null); } }} title={t(network === 'push' ? 'delivery.push' : 'delivery.pull')} footer={<><Button variant="secondary" onClick={() => { setNetwork(null); setPreview(null); }}>{t('dev.cancel')}</Button><Button disabled={disabled || !preview || !review?.branch || !review.head || network === 'pull' && (!!review.files.length || !!review.conflicts.length || 'merge_heads' in review && !!review.merge_heads.length)} onClick={() => void sendNetwork()}>{t(network === 'push' ? 'delivery.push' : 'delivery.pull')}</Button></>}>
      <div className="git-network"><p>{review?.path}</p><label>{t('delivery.remote')}<select value={remote} disabled={busy} onChange={event => { setRemote(event.target.value); setRemoteBranch(network === 'push' ? review?.branch ?? '' : ''); setPreview(null); }}><option value="">{t('gitPanel.chooseRemote')}</option>{details?.remotes.map(name => <option key={name} value={name}>{name}</option>)}</select></label><label>{t('delivery.branch')}<select value={remoteBranch} disabled={busy} onChange={event => { setRemoteBranch(event.target.value); setPreview(null); }}><option value="">{t('gitPanel.chooseBranch')}</option>{networkBranches.map(branch => <option key={branch} value={branch}>{branch}</option>)}</select></label>
        {network === 'pull' && remote && !networkBranches.length ? <p>{t('gitPanel.noCachedBranches')}</p> : null}
        <Button size="xs" disabled={disabled || !remote || !remoteBranch} onClick={() => void prepareNetwork()}>{t('gitPanel.previewTarget')}</Button>{preview ? <p className="git-network-preview">{preview.remote_display}<br /><code>{preview.remote_ref}</code><br />{review?.branch} · {review?.head}</p> : null}{error ? <p role="alert">{errorText(error)}</p> : null}
      </div>
    </Modal>
    <Modal open={historyPath !== null} onOpenChange={open => { if (!open) { ++historyTicket.current; setHistoryPath(null); } }} title={t('gitPanel.pathHistory')}><p className="git-history-path">{historyPath}</p>{historyError ? <div role="alert"><p>{errorText(historyError)}</p><Button size="xs" onClick={() => { if (historyPath !== null) void history(historyPath); }}>{t('gitPanel.retry')}</Button></div> : pathHistory ? historyList(pathHistory, pathTruncated, true) : <p role="status">{t('review.loading')}</p>}</Modal>
  </div>;
}

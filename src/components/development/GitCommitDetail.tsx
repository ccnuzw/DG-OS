import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { ArrowLeft, Check, ChevronDown, ChevronUp, Clock, Copy, File, GitCommitHorizontal, User, X } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { readDevelopmentGitCommit, readDevelopmentGitCommitDiff, type GitCommitDetails } from '../../stores/developmentGitCommit';
import type { DevDiff } from '../../stores/developmentReview';
import { parseUnifiedDiff } from './GitFileDiff';
import { SourceLines } from './ProjectReview';
import { errorText } from './shared';
import './file-workspace.css';
import './git-panel.css';
import './git-commit-detail.css';

interface Props { projectId: string; taskId: string | null; workspacePath: string; oid: string; onSelect(oid: string): void; onClose(): void }
export function GitCommitDetail(props: Props) {
  const epoch = useDevelopment(state => state.epoch);
  return <ScopedCommitDetail key={JSON.stringify([epoch, props.projectId, props.taskId, props.workspacePath, props.oid])} {...props} />;
}
function ScopedCommitDetail({ projectId, taskId, oid, onSelect, onClose }: Props) {
  const { t, locale } = useLocale();
  const [value, setValue] = useState<GitCommitDetails | null>(null), [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(true), [path, setPath] = useState<string | null>(null);
  const [copied, setCopied] = useState(false), [copyError, setCopyError] = useState(false);
  const request = useRef(0), close = useRef<HTMLButtonElement>(null), fileList = useRef<HTMLDivElement>(null), lastFile = useRef<HTMLButtonElement | null>(null);
  const refresh = useCallback(async () => {
    const ticket = ++request.current, epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    setBusy(true); setValue(null); setError(null);
    const result = await readDevelopmentGitCommit({ project_id: projectId, task_id: taskId, oid });
    if (ticket !== request.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    setBusy(false); if (result.ok) setValue(result.value); else setError(result.code);
  }, [projectId, taskId, oid]);
  useEffect(() => { void refresh(); return () => { ++request.current; }; }, [refresh]);
  useLayoutEffect(() => { close.current?.focus(); }, []);
  useLayoutEffect(() => { if (path === null && lastFile.current?.isConnected) lastFile.current.focus(); }, [path]);
  async function copy() {
    const epoch = useDevelopment.getState().epoch, ticket = request.current;
    try { await navigator.clipboard.writeText(oid); if (ticket === request.current && epoch === useDevelopment.getState().epoch) { setCopied(true); setCopyError(false); } }
    catch { if (ticket === request.current && epoch === useDevelopment.getState().epoch) setCopyError(true); }
  }
  const totals = value?.files.reduce((sum, file) => ({ added: sum.added + (file.added ?? 0), deleted: sum.deleted + (file.deleted ?? 0), unknown: sum.unknown + Number(file.added === null || file.deleted === null) }), { added: 0, deleted: 0, unknown: 0 });
  const message = value?.message ?? '', newline = message.indexOf('\n'), subject = newline < 0 ? message : message.slice(0, newline), body = newline < 0 ? '' : message.slice(newline + 1).replace(/^\n/, '');
  const date = value ? new Date(value.timestamp * 1000) : null;
  return <section className="git-commit-detail" data-testid="git-commit-detail" aria-label={t('gitPanel.commitDetails')} onKeyDown={event => { if (event.key === 'Escape' && !event.defaultPrevented) { event.preventDefault(); if (path !== null) setPath(null); else onClose(); } }}>
    <header className="git-detail-header"><GitCommitHorizontal size={15} aria-hidden="true" /><h2>{t('gitPanel.commitDetails')}</h2><button ref={close} type="button" data-testid="git-commit-close" className="git-icon" aria-label={t('gitPanel.closeDetails')} title={t('gitPanel.closeDetails')} onClick={onClose}><X size={16} aria-hidden="true" /></button></header>
    {busy ? <p className="git-feedback" role="status">{t('review.loading')}</p> : null}
    {error ? <div className="git-feedback git-error" role="alert"><p>{errorText(error)}</p><Button size="xs" variant="secondary" onClick={() => void refresh()}>{t('gitPanel.retry')}</Button></div> : null}
    {value ? <><div className="git-detail-summary"><h3>{subject || t('gitPanel.emptyMessage')}</h3>{body ? <p className="git-detail-body">{body}</p> : null}
      <dl className="git-detail-metadata">
        <div><dt><GitCommitHorizontal size={13} aria-hidden="true" /><span className="sr-only">{t('gitPanel.hash')}</span></dt><dd><button type="button" className="git-detail-hash" title={t('gitPanel.copyHash')} aria-label={t('gitPanel.copyHash')} onClick={() => void copy()}><code>{value.oid}</code>{copied ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}</button></dd></div>
        <div><dt><User size={13} aria-hidden="true" /><span className="sr-only">{t('gitPanel.author')}</span></dt><dd>{`${value.author} <${value.author_email}>`}</dd></div>
        <div><dt><Clock size={13} aria-hidden="true" /><span className="sr-only">{t('gitPanel.date')}</span></dt><dd><time dateTime={date?.toISOString()}>{date?.toLocaleString(locale)}</time></dd></div>
        <div><dt><GitCommitHorizontal size={13} aria-hidden="true" /><span className="sr-only">{t('gitPanel.parents')}</span></dt><dd className="git-detail-parents"><span>{t('gitPanel.parents')}:</span>{value.parents.length ? value.parents.map((parent, index) => <button key={parent} type="button" title={parent} aria-label={t('gitPanel.inspectParent', { oid: parent })} onClick={() => onSelect(parent)}><code>{parent.slice(0, 7)}</code>{index === 0 ? <span> ({t('gitPanel.firstParent')})</span> : null}</button>) : <span>{t('gitPanel.rootCommit')}</span>}</dd></div>
      </dl>{copied ? <span className="sr-only" role="status">{t('gitPanel.hashCopied')}</span> : null}{copyError ? <p className="git-error" role="alert">{t('gitPanel.copyHashFailed')}</p> : null}
      <p className="git-detail-comparison">{t(value.parents.length ? 'gitPanel.firstParentComparison' : 'gitPanel.emptyTreeComparison')}</p>
    </div>
      <div className="git-detail-files" ref={fileList} hidden={path !== null}><header><File size={14} aria-hidden="true" /><h3>{t('gitPanel.changedFiles')}</h3><span className="git-count">{value.files.length}{value.truncated ? '+' : ''}</span><span className="git-line-stats" aria-label={t('gitPanel.lineStats', { added: totals?.added ?? 0, deleted: totals?.deleted ?? 0 })}><span className="git-added">+{totals?.added}</span><span className="git-deleted">−{totals?.deleted}</span></span></header>
        {totals?.unknown ? <p className="git-feedback">{t('gitPanel.unknownStats', { count: totals.unknown })}</p> : null}{value.truncated ? <p className="git-feedback" role="status">{t('gitPanel.truncatedCommit')}</p> : null}
        <ol>{value.files.map(file => <li key={file.path}><button type="button" className="git-detail-file" data-commit-path={file.path} title={file.original_path ? `${file.original_path} → ${file.path}` : file.path} onClick={event => { lastFile.current = event.currentTarget; setPath(file.path); }}><span className={`git-status git-status-${file.status}`} aria-label={t('gitPanel.status', { status: file.status })}>{file.status}</span><span className="git-detail-path">{file.original_path ? `${file.original_path} → ` : ''}{file.path}</span><span className="git-line-stats" title={file.added === null || file.deleted === null ? t('gitPanel.binaryStats') : undefined}><span className="git-added">{file.added === null ? '—' : `+${file.added}`}</span><span className="git-deleted">{file.deleted === null ? '—' : `−${file.deleted}`}</span></span></button></li>)}</ol>{!value.files.length ? <p className="git-feedback">{t('gitPanel.noChangedFiles')}</p> : null}
      </div>
      {path !== null ? <HistoricalDiff key={path} projectId={projectId} taskId={taskId} oid={oid} path={path} onBack={() => setPath(null)} /> : null}
    </> : null}
  </section>;
}
function HistoricalDiff({ projectId, taskId, oid, path, onBack }: { projectId: string; taskId: string | null; oid: string; path: string; onBack(): void }) {
  const { t } = useLocale();
  const [value, setValue] = useState<DevDiff | null>(null), [error, setError] = useState<string | null>(null), [busy, setBusy] = useState(true);
  const [mode, setMode] = useState('unified'), [selected, setSelected] = useState(0);
  const request = useRef(0), root = useRef<HTMLDivElement>(null), back = useRef<HTMLButtonElement>(null);
  const refresh = useCallback(async () => {
    const ticket = ++request.current, epoch = useDevelopment.getState().epoch, generation = contextGeneration();
    setBusy(true); setError(null); setValue(null);
    const result = await readDevelopmentGitCommitDiff({ project_id: projectId, task_id: taskId, oid, path });
    if (ticket !== request.current || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
    setBusy(false); if (result.ok) setValue(result.value); else setError(result.code);
  }, [projectId, taskId, oid, path]);
  useEffect(() => { void refresh(); return () => { ++request.current; }; }, [refresh]);
  useLayoutEffect(() => { back.current?.focus(); }, []);
  const rows = useMemo(() => parseUnifiedDiff(value?.diff ?? ''), [value?.diff]);
  const count = rows.filter(row => row.kind === 'hunk').length;
  function navigate(next: number) { setSelected(next); root.current?.querySelector(`[data-hunk="${next}"]`)?.scrollIntoView?.({ block: 'nearest' }); }
  return <div className="git-historical-diff" data-testid="dev-git-commit-diff"><header className="git-detail-diff-header"><Button ref={back} size="xs" variant="ghost" onClick={onBack}><ArrowLeft size={14} aria-hidden="true" />{t('gitPanel.backToFiles')}</Button><span title={path}>{path}</span></header>
    <p className="git-detail-readonly">{t('gitPanel.historicalDiff')} <code>{oid.slice(0, 7)}</code></p>
    <nav className="file-diff-toolbar" aria-label={t('review.diffView')}>{['unified', 'before', 'after'].map(item => <Button key={item} size="xs" variant={mode === item ? 'secondary' : 'ghost'} aria-pressed={mode === item} onClick={() => setMode(item)}>{t(`review.${item}`)}</Button>)}</nav>
    {mode === 'unified' ? <div className="file-diff-toolbar"><Button size="xs" variant="ghost" aria-label={t('fileWorkspace.previousHunk')} disabled={!count || selected === 0} onClick={() => navigate(selected - 1)}><ChevronUp size={14} aria-hidden="true" /></Button><span role="status">{t('fileWorkspace.hunks', { current: count ? selected + 1 : 0, count })}</span><Button size="xs" variant="ghost" aria-label={t('fileWorkspace.nextHunk')} disabled={!count || selected >= count - 1} onClick={() => navigate(selected + 1)}><ChevronDown size={14} aria-hidden="true" /></Button></div> : null}
    {busy ? <p className="git-feedback" role="status">{t('review.loading')}</p> : null}{error ? <div className="git-feedback git-error" role="alert"><p>{errorText(error)}</p><Button size="xs" variant="secondary" onClick={() => void refresh()}>{t('gitPanel.retry')}</Button></div> : null}
    {value?.reason ? <p className="git-feedback" role="status">{value.reason === 'git_special_entry' ? t('gitPanel.specialEntry') : ['file_binary', 'file_sensitive', 'file_too_large', 'file_access_denied', 'file_not_found', 'file_symlink', 'unavailable'].includes(value.reason) ? t(`review.reason.${value.reason}`) : errorText(value.reason)}</p> : null}
    {value && !value.reason && mode !== 'unified' ? value[mode === 'before' ? 'before' : 'after'] !== null ? <SourceLines text={value[mode === 'before' ? 'before' : 'after']!} label={t(`review.${mode}`)} /> : <p className="git-feedback">{t('gitPanel.absentVersion')}</p> : null}
    {value && !value.reason && mode === 'unified' ? value.diff ? <div ref={root} className="file-unified-diff" tabIndex={0} role="region" aria-label={t('review.unified')}>{rows.map((row, index) => <div key={index} className={`file-diff-line file-diff-${row.kind}`} data-hunk={row.kind === 'hunk' ? row.hunk : undefined} data-selected={row.hunk === selected}><span className="file-diff-number" aria-hidden="true">{row.before}</span><span className="file-diff-number" aria-hidden="true">{row.after}</span><code>{row.text || ' '}</code></div>)}</div> : <p className="git-feedback">{t('delivery.emptyDiff')}</p> : null}
  </div>;
}

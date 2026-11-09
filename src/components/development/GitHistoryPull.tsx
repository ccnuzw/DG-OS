import { useEffect, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { readDevelopmentGitHistory, previewDevelopmentGitPull, type GitAction, type GitHistoryPage, type GitPullPreview, type GitReview } from '../../stores/developmentDelivery';
import { confirmAction, errorText } from './shared';
import { readDevelopmentProjectGitHistory, type ProjectGitOverview } from '../../stores/developmentExplorer';

export function GitHistoryPull({ projectId, review, busy, blocked, act, refreshKey }: { projectId?: string; review: GitReview | ProjectGitOverview; busy: boolean; blocked: boolean; act: (action: GitAction) => Promise<void>; refreshKey?: string | number }) {
  const taskReview = 'review_token' in review ? review : null;
  const { locale, t } = useLocale();
  const [page, setPage] = useState<GitHistoryPage | null>(null);
  const [remote, setRemote] = useState(''); const [branch, setBranch] = useState('');
  const [preview, setPreview] = useState<GitPullPreview | null>(null);
  const [loading, setLoading] = useState(false); const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true); const ticket = useRef(0);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++ticket.current; }; }, []);
  useEffect(() => { ++ticket.current; setPreview(null); setLoading(false); }, [taskReview?.task_id, projectId, review.path, review.head, taskReview?.review_token, refreshKey, remote, branch]);
  useEffect(() => { setPage(null); }, [taskReview?.task_id, projectId, review.path, review.head]);
  function guard() { const n = ++ticket.current, epoch = useDevelopment.getState().epoch, generation = contextGeneration(); return () => mounted.current && n === ticket.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration(); }
  async function history(more: boolean) {
    const valid = guard(); setLoading(true); setError(null);
    const input = { start_oid: more ? page?.anchor : review.head, cursor: more ? page?.next_cursor : null, limit: 30 };
    const result = taskReview ? await readDevelopmentGitHistory({ ...input, task_id: taskReview.task_id }) : projectId ? await readDevelopmentProjectGitHistory({ ...input, project_id: projectId }) : { ok: false as const, code: 'project_not_found' };
    if (!valid()) return;
    if (result.ok) setPage(old => more && old ? { ...result.value, commits: [...old.commits, ...result.value.commits] } : result.value); else setError(result.code);
    setLoading(false);
  }
  async function prepare() {
    if (!taskReview) return;
    const valid = guard(); setLoading(true); setError(null);
    const result = await previewDevelopmentGitPull(taskReview.task_id, remote, branch);
    if (!valid()) return;
    if (result.ok) setPreview(result.value); else setError(result.code);
    setLoading(false);
  }
  async function pull() {
    if (!taskReview || !preview || preview.review_token !== taskReview.review_token) return;
    const valid = guard();
    if (!await confirmAction(t('delivery.pull'), t('delivery.pullTrust') + '\n' + review.path + '\n' + preview.remote_display + '\n' + preview.remote_ref, t('delivery.pull')) || !valid()) return;
    await act({ kind: 'pull', remote: preview.remote, branch: preview.branch, preview_token: preview.preview_token, trust_directory: true, trust_network: true });
    if (valid()) setPreview(null);
  }
  return <section className="dev-review-stack" aria-label={t('delivery.historyPull')}>
    <div className="dev-review-toolbar"><h5 className="m-0 text-xs">{t('delivery.gitHistory')}</h5><Button size="xs" variant="secondary" disabled={loading || busy} onClick={() => void history(false)}>{t('delivery.openGitHistory')}</Button></div>
    {error ? <p role="alert" className="text-xs text-status-error">{errorText(error)}</p> : null}
    {loading ? <p role="status" className="text-xs">{t('review.loading')}</p> : null}
    {page ? <details className="dev-technical-details" open><summary>{t('delivery.gitHistory')}</summary><p className="m-0 break-all text-xs">{page.anchor}</p>{!page.commits.length ? <p className="text-xs">{t('delivery.noCommits')}</p> : <ol className="dev-git-history-list">{page.commits.map(entry => <li key={entry.oid} className="dev-review-stack"><p className="m-0 break-words text-xs">{entry.subject}</p><p className="m-0 text-xs text-ink-secondary">{entry.author} · {new Date(entry.timestamp * 1000).toLocaleString(locale)}</p><details><summary><code>{entry.oid.slice(0, 8)}</code></summary><code className="break-all text-xs">{entry.oid}</code><p className="m-0 break-all text-xs text-ink-secondary">{entry.parents.join(' ')}</p></details></li>)}</ol>}{page.next_cursor ? <Button size="xs" variant="secondary" disabled={loading || busy} onClick={() => void history(true)}>{t('delivery.moreGitHistory')}</Button> : null}</details> : null}
    {taskReview ? <details className="dev-technical-details"><summary>{t('delivery.previewPull')}</summary>
    <Input label={t('delivery.pullRemote')} value={remote} onChange={event => { ++ticket.current; setPreview(null); setRemote(event.target.value); }} />
    <Input label={t('delivery.pullBranch')} value={branch} onChange={event => { ++ticket.current; setPreview(null); setBranch(event.target.value); }} />
    <Button size="xs" variant="secondary" disabled={busy || loading || blocked || !remote || !branch} onClick={() => void prepare()}>{t('delivery.previewPull')}</Button>
    {preview ? <div className="dev-review-stack"><p className="m-0 break-all text-xs">{preview.remote_display} · {preview.remote_ref}</p><p className="m-0 break-all text-xs">{review.branch} · {review.head}</p><Button size="xs" disabled={busy || loading || blocked || !review.branch || !review.head || review.files.length > 0 || review.conflicts.length > 0 || taskReview.merge_heads.length > 0} onClick={() => void pull()}>{t('delivery.pull')}</Button></div> : null}
    </details> : null}
  </section>;
}

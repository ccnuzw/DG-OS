import { useEffect, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration } from '../../stores/developmentSelection';
import { readDevelopmentGitHistory, previewDevelopmentGitPull, type GitAction, type GitHistoryPage, type GitPullPreview, type GitReview } from '../../stores/developmentDelivery';
import { confirmAction, errorText } from './shared';

export function GitHistoryPull({ review, busy, blocked, act }: { review: GitReview; busy: boolean; blocked: boolean; act: (action: GitAction) => Promise<void> }) {
  const { locale, t } = useLocale();
  const [page, setPage] = useState<GitHistoryPage | null>(null);
  const [remote, setRemote] = useState(''); const [branch, setBranch] = useState('');
  const [preview, setPreview] = useState<GitPullPreview | null>(null);
  const [loading, setLoading] = useState(false); const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true); const ticket = useRef(0);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++ticket.current; }; }, []);
  useEffect(() => { ++ticket.current; setPreview(null); setLoading(false); }, [review.review_token, remote, branch]);
  function guard() { const n = ++ticket.current, epoch = useDevelopment.getState().epoch, generation = contextGeneration(); return () => mounted.current && n === ticket.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration(); }
  async function history(more: boolean) {
    const valid = guard(); setLoading(true); setError(null);
    const result = await readDevelopmentGitHistory({ task_id: review.task_id, start_oid: more ? page?.anchor : review.head, cursor: more ? page?.next_cursor : null, limit: 30 });
    if (!valid()) return;
    if (result.ok) setPage(old => more && old ? { ...result.value, commits: [...old.commits, ...result.value.commits] } : result.value); else setError(result.code);
    setLoading(false);
  }
  async function prepare() {
    const valid = guard(); setLoading(true); setError(null);
    const result = await previewDevelopmentGitPull(review.task_id, remote, branch);
    if (!valid()) return;
    if (result.ok) setPreview(result.value); else setError(result.code);
    setLoading(false);
  }
  async function pull() {
    if (!preview || preview.review_token !== review.review_token) return;
    const valid = guard();
    if (!await confirmAction(t('delivery.pull'), t('delivery.pullTrust') + '\n' + review.path + '\n' + preview.remote_display + '\n' + preview.remote_ref, t('delivery.pull')) || !valid()) return;
    await act({ kind: 'pull', remote: preview.remote, branch: preview.branch, preview_token: preview.preview_token, trust_directory: true, trust_network: true });
    if (valid()) setPreview(null);
  }
  return <section className="dev-review-stack" aria-label={t('delivery.historyPull')}>
    <div className="dev-review-toolbar"><h5 className="m-0 text-xs">{t('delivery.gitHistory')}</h5><Button size="xs" variant="secondary" disabled={loading || busy} onClick={() => void history(false)}>{t('delivery.openGitHistory')}</Button></div>
    {error ? <p role="alert" className="text-xs text-status-error">{errorText(error)}</p> : null}
    {loading ? <p role="status" className="text-xs">{t('review.loading')}</p> : null}
    {page ? <><p className="m-0 break-all text-xs">{page.anchor}</p>{!page.commits.length ? <p className="text-xs">{t('delivery.noCommits')}</p> : <ol className="m-0 list-none p-0">{page.commits.map(entry => <li key={entry.oid} className="dev-review-stack"><p className="m-0 break-words text-xs">{entry.subject}</p><p className="m-0 text-xs text-ink-secondary">{entry.author} · {new Date(entry.timestamp * 1000).toLocaleString(locale)}</p><code className="break-all text-xs">{entry.oid}</code><p className="m-0 break-all text-xs text-ink-secondary">{entry.parents.join(' ')}</p></li>)}</ol>}{page.next_cursor ? <Button size="xs" variant="secondary" disabled={loading || busy} onClick={() => void history(true)}>{t('delivery.moreGitHistory')}</Button> : null}</> : null}
    <Input label={t('delivery.pullRemote')} value={remote} onChange={event => { ++ticket.current; setPreview(null); setRemote(event.target.value); }} />
    <Input label={t('delivery.pullBranch')} value={branch} onChange={event => { ++ticket.current; setPreview(null); setBranch(event.target.value); }} />
    <Button size="xs" variant="secondary" disabled={busy || loading || blocked || !remote || !branch} onClick={() => void prepare()}>{t('delivery.previewPull')}</Button>
    {preview ? <div className="dev-review-stack"><p className="m-0 break-all text-xs">{preview.remote_display} · {preview.remote_ref}</p><p className="m-0 break-all text-xs">{review.branch} · {review.head}</p><Button size="xs" disabled={busy || loading || blocked || !review.branch || !review.head || review.files.length > 0 || review.conflicts.length > 0 || review.merge_heads.length > 0} onClick={() => void pull()}>{t('delivery.pull')}</Button></div> : null}
  </section>;
}

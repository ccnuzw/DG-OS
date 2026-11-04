import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { DevelopmentSession } from '../../stores/developmentStore';

// Saved sessions without a task. Shown whether or not a project is selected, so history stays reachable. Classifying
// needs a selected task; without one the action is disabled and no task is invented.
export function UnclassifiedPanel({ sessions, taskId, busy, onClassify }: { sessions: DevelopmentSession[]; taskId: string | null; busy: boolean; onClassify(session: DevelopmentSession): void }) {
  const { t } = useLocale();
  return <section aria-label={t('dev.unclassified')} className="dev-panel" data-testid="dev-unclassified">
    <h2 className="m-0 text-sm font-semibold">{t('dev.unclassified')}</h2>
    <p className="m-0 text-xs text-muted-foreground">{t('dev.unclassifiedHint')}</p>
    {!sessions.length ? <p className="m-0 text-xs text-muted-foreground">{t('dev.noUnclassified')}</p> : null}
    {sessions.map(session => <div key={session.id} className="flex items-center gap-2 text-xs" data-testid={`dev-session-unclassified-${session.id}`}>
      <span className="min-w-0 flex-1 truncate">{session.engine === 'codex' ? 'Codex' : 'Claude'} · {session.workspace}</span>
      <Button size="xs" variant="secondary" data-testid={`dev-session-classify-${session.id}`} disabled={busy || !taskId} onClick={() => onClassify(session)}>{t('dev.classifyTo')}</Button>
    </div>)}
  </section>;
}

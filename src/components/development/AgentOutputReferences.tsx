import { useEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { AgentEvent } from '../../stores/agentRunnerStore';
import { agentEventFailed, agentOutputReferences, type AgentReferenceActions } from './eventOutputReferences';

export function AgentOutputReferences({ event, toolNames, onOpenReference, onSelectRun }: AgentReferenceActions & { event: AgentEvent; toolNames: ReadonlyMap<string, string> }) {
  const { t } = useLocale();
  const { references, unresolved } = agentOutputReferences(event, toolNames);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const current = useRef(true);
  useEffect(() => { current.current = true; return () => { current.current = false; }; }, []);
  const run = agentEventFailed(event) && event.run;
  if (!references.length && !unresolved && !run) return null;
  return <section data-testid={`agent-event-references-${event.seq}`} className="flex min-w-0 flex-wrap items-center gap-2 py-2 text-xs" aria-label={t('agent.references')}>
    {references.map((reference, index) => <div key={`${reference.path}:${reference.line ?? ''}:${index}`} data-reference-path={reference.path} data-reference-line={reference.line} className="flex min-w-0 flex-wrap items-center gap-2"><code className="break-all">{reference.path}{reference.line ? `:${reference.line}` : ''}</code>{(['file', ...(reference.diff ? ['diff'] : [])] as ('file' | 'diff')[]).map(mode => <Button key={mode} title={mode === 'diff' ? t('agentExecution.diskDiff') : undefined} data-testid={`agent-reference-${event.seq}-${index}-${mode}`} data-reference-mode={mode} size="xs" variant="secondary" disabled={busy || !onOpenReference} onClick={async () => {
      if (busy || !onOpenReference) return;
      setBusy(true); setFailed(false);
      try { await onOpenReference(reference, mode); } catch { if (current.current) setFailed(true); }
      finally { if (current.current) setBusy(false); }
    }}>{t(mode === 'file' ? 'agent.openFile' : 'agent.openDiff')}</Button>)}</div>)}
    {run ? onSelectRun ? <Button data-testid={`agent-reference-${event.seq}-run`} data-run-id={run} size="xs" variant="secondary" onClick={() => onSelectRun(run)}>{t('agent.openRun')}</Button> : <span>{t('agent.referenceUnavailable')}</span> : null}
    {unresolved ? <span className="text-ink-secondary">{t('agent.referenceUnresolved')}</span> : null}
    {references.length > 0 && !onOpenReference ? <span className="text-ink-secondary">{t('agent.referenceUnavailable')}</span> : null}
    {references.some(reference => reference.diff) ? <span className="text-ink-secondary">{t('agentExecution.diskReferenceBoundary')}</span> : null}
    {failed ? <span role="alert" className="text-status-error">{t('agent.referenceUnavailable')}</span> : null}
  </section>;
}

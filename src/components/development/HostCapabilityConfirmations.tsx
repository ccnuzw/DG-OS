import { useEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { listCapabilityConfirmations, respondCapabilityConfirmation, type CapabilityConfirmation } from '../../stores/developmentCapabilities';
import { hostCode } from '../../stores/hostError';
import { errorText } from './shared';

export interface HostConfirmationProjection {
  pending: CapabilityConfirmation[]; error: string | null; busy: string | null;
  respond(item: CapabilityConfirmation, approved: boolean): Promise<void>;
}
// Polling only projects host requests. Unmounting never denies a request or cancels its runtime.
export function useHostCapabilityConfirmations(sessionId: string | null) {
  const epoch = useAgentRunner(state => state.epoch);
  const devEpoch = useDevelopment(state => state.epoch);
  const { generation } = useContextSelection();
  const selected = useAgentRunner(state => state.selected);
  const status = useAgentRunner(state => state.session?.id === sessionId ? state.session.status : null);
  const selection = useDevelopment(state => state.selection);
  const [pending, setPending] = useState<CapabilityConfirmation[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const currentPending = useRef<CapabilityConfirmation[]>([]);
  const alive = useRef(false);
  const projection = useRef('');
  const scope = JSON.stringify([sessionId, epoch, devEpoch, generation, selection.project_id, selection.task_id]);
  const responding = useRef<string | null>(null);
  const consumed = useRef(new Set<string>());
  useEffect(() => {
    let disposed = false;
    let timer: number | undefined;
    alive.current = true; currentPending.current = []; consumed.current.clear(); setPending([]); setError(null); setBusy(null); responding.current = null;
    if (!sessionId || selected !== sessionId || status === 'closed') return () => { alive.current = false; };
    async function poll() {
      try {
        const items = await listCapabilityConfirmations(sessionId!);
        if (disposed || epoch !== useAgentRunner.getState().epoch || devEpoch !== useDevelopment.getState().epoch || generation !== contextGeneration() || useAgentRunner.getState().selected !== sessionId) return;
        currentPending.current = items.filter(item => item.session_id === sessionId && (!selection.project_id || item.project_id === selection.project_id) && (!selection.task_id || item.task_id === selection.task_id) && !consumed.current.has(item.id));
        setPending(currentPending.current); setError(null);
        projection.current = scope;
      } catch (failure) {
        if (disposed || epoch !== useAgentRunner.getState().epoch || devEpoch !== useDevelopment.getState().epoch || generation !== contextGeneration()) return;
        // A failed read cannot leave a stale approval actionable.
        currentPending.current = []; setPending([]); setError(hostCode(failure));
      }
      if (!disposed) timer = window.setTimeout(() => void poll(), 700);
    }
    void poll();
    return () => { disposed = true; alive.current = false; currentPending.current = []; window.clearTimeout(timer); };
  }, [sessionId, epoch, devEpoch, generation, selected, status === 'closed', selection.project_id, selection.task_id]);
  async function respond(item: CapabilityConfirmation, approved: boolean) {
    if (!sessionId || !alive.current || responding.current || epoch !== useAgentRunner.getState().epoch || devEpoch !== useDevelopment.getState().epoch || generation !== contextGeneration() || useAgentRunner.getState().selected !== sessionId || useAgentRunner.getState().session?.status === 'closed' || !currentPending.current.some(value => value.id === item.id && value.run_id === item.run_id && value.schema_hash === item.schema_hash)) return;
    responding.current = item.id; setBusy(item.id); setError(null);
    // Consume locally before IPC: neither a rerender nor a stale event can replay this confirmation.
    consumed.current.add(item.id);
    currentPending.current = currentPending.current.filter(value => value.id !== item.id);
    setPending(currentPending.current);
    try { await respondCapabilityConfirmation(sessionId, item.id, approved); }
    catch (failure) { if (alive.current && epoch === useAgentRunner.getState().epoch && devEpoch === useDevelopment.getState().epoch && generation === contextGeneration()) setError(hostCode(failure)); }
    finally { if (alive.current && epoch === useAgentRunner.getState().epoch && devEpoch === useDevelopment.getState().epoch && generation === contextGeneration()) { responding.current = null; setBusy(null); } }
  }
  return { pending: projection.current === scope && sessionId === selected && status !== 'closed' ? pending : [], error, busy, respond };
}

export function HostCapabilityConfirmations({ sessionId }: { sessionId: string }) {
  const confirmations = useHostCapabilityConfirmations(sessionId);
  return <HostConfirmationRequests confirmations={confirmations} />;
}

export function HostConfirmationRequests({ confirmations }: { confirmations: HostConfirmationProjection }) {
  const { t } = useLocale();
  const { pending, error, busy, respond } = confirmations;
  return <>
    {error ? <p role="alert" className="text-xs text-status-error">{t('capabilities.confirmationFailure')} {errorText(error)}</p> : null}
    {pending.map(item => <section key={item.id} className="dev-runtime-fields" aria-label={t('capabilities.hostApproval')}>
      <strong className="text-sm">{t('capabilities.hostApproval')}</strong>
      <p className="m-0 text-xs text-ink-secondary">{t('capabilities.mcpBoundary')}</p>
      <dl className="m-0 grid gap-1 text-xs"><dt>{t('capabilities.server')}</dt><dd className="m-0 break-all">{item.server_id}</dd><dt>{t('capabilities.tool')}</dt><dd className="m-0 break-all">{item.tool_name}</dd></dl>
      <details className="dev-technical-details"><summary>{t('agent.details')}</summary><dl className="m-0 grid gap-1"><dt>{t('capabilities.schema')}</dt><dd className="m-0 break-all">{item.schema_hash}</dd><dt>{t('capabilities.run')}</dt><dd className="m-0 break-all">{item.run_id}</dd></dl></details>
      <strong className="text-xs">{t('capabilities.arguments')}</strong><pre className="dev-event-text">{JSON.stringify(item.arguments, null, 2)}</pre>
      <div className="flex flex-wrap gap-2"><Button size="xs" disabled={Boolean(busy)} onClick={() => void respond(item, true)}>{t('agent.allow')}</Button><Button size="xs" variant="secondary" disabled={Boolean(busy)} onClick={() => void respond(item, false)}>{t('agent.deny')}</Button></div>
    </section>)}
  </>;
}

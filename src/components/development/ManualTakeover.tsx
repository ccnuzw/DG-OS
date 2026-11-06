import { useEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { takeoverKey, useDevelopmentTakeover } from '../../stores/developmentTakeover';
import { confirmAction, errorText } from './shared';

interface Props { projectId: string; taskId: string; onReleased?: () => void }
export function ManualTakeover(props: Props) {
  const epoch = useDevelopment(state => state.epoch);
  const { generation } = useContextSelection();
  return <ScopedTakeover key={`${epoch}:${generation}:${props.projectId}:${props.taskId}`} {...props} />;
}
function ScopedTakeover({ projectId, taskId, onReleased }: Props) {
  const { t } = useLocale();
  const entry = useDevelopmentTakeover(state => state.entries[takeoverKey(projectId, taskId)]);
  const tasks = useDevelopment(state => state.tasks);
  const alive = useRef(true);
  const [confirming, setConfirming] = useState(false);
  useEffect(() => {
    alive.current = true;
    void useDevelopmentTakeover.getState().refresh(projectId, taskId);
    return () => { alive.current = false; };
  }, [projectId, taskId]);
  const view = entry?.view;
  const busy = entry?.busy || confirming;
  async function close() {
    if (!view || busy || !view.can_close) return;
    const epoch = useDevelopment.getState().epoch;
    const generation = contextGeneration();
    const details = view.sessions.map(session => `${session.engine} · ${session.id}\n${t('takeover.task')}: ${tasks.find(task => task.id === session.task_id)?.goal ?? session.task_id ?? t('dev.unclassified')}`).join('\n\n');
    setConfirming(true);
    const confirmed = await confirmAction(t('takeover.close'), `${t('takeover.warning')}\n\n${view.directory}\n\n${details}`, t('takeover.close'));
    if (!alive.current) return;
    setConfirming(false);
    if (!confirmed || epoch !== useDevelopment.getState().epoch || generation !== contextGeneration() || useDevelopmentTakeover.getState().entries[takeoverKey(projectId, taskId)]?.view !== view) return;
    const released = await useDevelopmentTakeover.getState().close(projectId, taskId, view);
    if (released && alive.current && epoch === useDevelopment.getState().epoch && generation === contextGeneration()) onReleased?.();
  }
  return <section className="dev-panel dev-review-stack" aria-label={t('takeover.title')} aria-busy={Boolean(busy)} data-testid="dev-manual-takeover">
    <div className="dev-review-toolbar"><h4 className="m-0 text-xs font-semibold">{t('takeover.title')}</h4><Button size="xs" variant="secondary" disabled={busy} onClick={() => void useDevelopmentTakeover.getState().refresh(projectId, taskId)}>{t('dev.inspect')}</Button></div>
    {busy ? <p role="status" className="m-0 text-xs">{t('dev.busy')}</p> : null}
    {entry?.error ? <p role="alert" className="m-0 text-xs text-status-error">{errorText(entry.error)}</p> : null}
    {view ? <>
      <p className="m-0 break-all text-xs">{view.directory}</p>
      {view.sessions.length ? <ul className="m-0 pl-4 text-xs">{view.sessions.map(session => <li key={session.id} className="break-all"><span>{session.engine} · {session.id}</span><br /><span>{t('takeover.task')}: {tasks.find(task => task.id === session.task_id)?.goal ?? session.task_id ?? t('dev.unclassified')}</span><br /><span>{t('takeover.status')}: {t(`agent.state.${session.status}`)}</span></li>)}</ul> : null}
      {view.restricted ? <p className="dev-evidence-warning">{t('takeover.restricted')}</p> : null}
      {view.host_busy ? <p className="dev-evidence-warning">{t('takeover.hostBusy')}</p> : null}
      {entry?.released ? <p role="status" className="m-0 text-xs">{t('takeover.released')}</p> : !view.sessions.length && !view.restricted && !view.host_busy ? <p role="status" className="m-0 text-xs">{t('takeover.clear')}</p> : null}
      {view.sessions.length ? <><p className="m-0 text-xs">{t('takeover.hint')}</p><div className="dev-danger-actions"><Button size="xs" variant="danger" disabled={busy || !view.can_close} onClick={() => void close()}>{t('takeover.close')}</Button></div></> : null}
    </> : null}
  </section>;
}

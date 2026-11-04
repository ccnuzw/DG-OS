import { Button, Input, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { Engine, LiveSessions } from '../../stores/agentRunnerStore';
import type { DevelopmentSession, DevelopmentTask, Project } from '../../stores/developmentStore';
import type { CapabilityRevision } from '../../stores/developmentCapabilities';

interface Props {
  project: Project;
  task: DevelopmentTask | null;
  sessions: DevelopmentSession[];
  live: LiveSessions;
  selectedId: string | null;
  trusted: boolean;
  onTrusted(value: boolean): void;
  engine: Engine;
  onEngine(value: Engine): void;
  model: string;
  onModel(value: string): void;
  busy: boolean;
  projectCapabilities: CapabilityRevision | null;
  selectedCapabilities: CapabilityRevision | null;
  capabilitiesLoading: boolean;
  capabilitiesError: string | null;
  onCapabilities(value: CapabilityRevision | null): void;
  onStart(): void;
  onResume(session: DevelopmentSession): void;
}

// Sessions of the selected task. Restoring a saved session never starts a CLI by itself: each restore is an explicit
// click made with trust confirmed. A session whose runtime is still live is read from its snapshot instead.
export function SessionPanel(props: Props) {
  const { t } = useLocale();
  const { project, task, sessions, live, selectedId, trusted, busy } = props;
  const canStart = Boolean(task) && !project.archived && trusted && !busy;
  return <section className="dev-panel" aria-label={t('dev.sessions')} data-testid="dev-session-list">
    <header className="dev-panel-header"><h2 className="m-0 text-sm font-semibold">{t('dev.sessions')}</h2></header>
    {project.archived ? <p role="alert" className="m-0 text-xs text-destructive">{t('dev.error.project_archived')}</p> : null}
    <label className="flex items-start gap-2 text-xs"><input type="checkbox" data-testid="dev-trust" checked={trusted} onChange={event => props.onTrusted(event.target.checked)} /><span>{t('dev.trustProject')}</span></label>
    <p className="m-0 text-xs text-muted-foreground">{t('dev.trustHint')} {t('dev.resumeNotStarted')}</p>
    <Select label={t('capabilities.newSessionSource')} value={props.selectedCapabilities ? String(props.selectedCapabilities.revision) : 'native'} disabled={busy} onChange={value => props.onCapabilities(value === 'native' ? null : props.projectCapabilities?.revision === Number(value) ? props.projectCapabilities : props.selectedCapabilities)} options={[{ value: 'native', label: t('capabilities.native') }, ...(props.selectedCapabilities ? [{ value: String(props.selectedCapabilities.revision), label: t('capabilities.projectRevision', { revision: props.selectedCapabilities.revision }) }] : []), ...(props.projectCapabilities && props.projectCapabilities.revision !== props.selectedCapabilities?.revision ? [{ value: String(props.projectCapabilities.revision), label: t('capabilities.projectRevision', { revision: props.projectCapabilities.revision }) }] : [])]} />
    {props.capabilitiesLoading ? <p role="status" className="text-xs text-muted-foreground">{t('capabilities.loading')}</p> : null}
    {props.capabilitiesError ? <p role="alert" className="text-xs text-destructive">{props.capabilitiesError}</p> : null}
    {props.selectedCapabilities ? <p className="m-0 break-all text-xs">{t('capabilities.fixedModel', { engine: props.selectedCapabilities.configuration.engine === 'codex' ? 'Codex' : 'Claude', model: props.selectedCapabilities.configuration.provider.source === 'dgos' ? props.selectedCapabilities.configuration.provider.model_id : t('capabilities.native') })}</p> : <>
      <Select label={t('agent.engine')} value={props.engine} onChange={value => props.onEngine(value === 'claude' ? 'claude' : 'codex')} options={[{ value: 'codex', label: t('agent.engine.codex') }, { value: 'claude', label: t('agent.engine.claude') }]} />
      <Input label={t('agent.model')} value={props.model} onChange={event => props.onModel(event.target.value)} />
    </>}
    <p className="m-0 text-xs text-muted-foreground">{t('capabilities.restorePinned')}</p>
    {task ? <Button size="xs" data-testid={`dev-session-new-${props.selectedCapabilities?.configuration.engine ?? props.engine}`} disabled={!canStart} onClick={props.onStart}>{t('dev.newSession')}</Button> : <p className="m-0 text-xs text-muted-foreground">{t('dev.needTaskForSession')}</p>}
    {task && !sessions.length ? <p className="m-0 text-xs text-muted-foreground">{t('dev.noSessions')}</p> : null}
    <nav aria-label={t('dev.sessions')} className="grid gap-1">
      {sessions.map(session => {
        const status = live[session.id];
        return <div key={session.id} className="flex items-center gap-2 text-xs" data-testid={`dev-session-item-${session.id}`}>
          <span className="min-w-0 flex-1 truncate" aria-current={selectedId === session.id ? 'true' : undefined}>{session.engine === 'codex' ? 'Codex' : 'Claude'} · {status ? t(`agent.state.${status}`) : session.id.slice(0, 8)}</span>
          <Button size="xs" variant="secondary" disabled={busy || (!status && !trusted)} onClick={() => props.onResume(session)}>{status ? t('dev.resume') : t('dev.trustResume')}</Button>
        </div>;
      })}
    </nav>
  </section>;
}

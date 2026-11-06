import { useEffect, useId, useState } from 'react';
import { Button, Input, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { Engine, LiveSessions } from '../../stores/agentRunnerStore';
import type { DevelopmentSession, DevelopmentTask, Project } from '../../stores/developmentStore';
import type { CapabilityRevision } from '../../stores/developmentCapabilities';

interface Props {
  project: Project;
  task: DevelopmentTask | null;
  workspacePath?: string | null;
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

// Browsing a session is always explicit. Starting or restoring a CLI still requires directory trust.
export function SessionPanel(props: Props) {
  const { t } = useLocale();
  const { project, task, sessions, live, selectedId, trusted, busy } = props;
  const [setupOpen, setSetupOpen] = useState(Boolean(task) && sessions.length === 0);
  const [search, setSearch] = useState('');
  const query = search.trim().toLocaleLowerCase();
  const matchingIds = new Set(sessions.filter(session => [session.engine, session.workspace, session.id, session.native_id ?? '', session.model ?? ''].some(value => value.toLocaleLowerCase().includes(query))).map(session => session.id));
  const visible = sessions.filter(session => session.id === selectedId || matchingIds.has(session.id));
  const trustHintId = useId();
  const setupId = useId();
  useEffect(() => { setSetupOpen(Boolean(task) && sessions.length === 0); }, [project.id, task?.id, sessions.length]);
  useEffect(() => { setSearch(''); }, [project.id, task?.id]);
  const canStart = Boolean(task) && Boolean(props.workspacePath) && !project.archived && trusted && !busy;
  return <section className="dev-panel dev-session-panel" aria-label={t('dev.sessions')} data-testid="dev-session-list">
    <header className="dev-panel-header"><h2 className="m-0 text-sm font-semibold">{t('dev.sessions')}</h2></header>
    {project.archived ? <p role="alert" className="m-0 text-xs text-status-error">{t('dev.error.project_archived')}</p> : null}
    {!task ? <p className="m-0 text-xs text-ink-secondary">{t('dev.needTaskForSession')}</p> : null}
    {task && !sessions.length ? <p className="m-0 text-xs text-ink-secondary">{t('dev.noSessions')}</p> : null}
    {sessions.length ? <div className="dev-nav-search"><Input type="search" data-testid="dev-session-search" label={t('workbenchNavigation.searchSessions')} placeholder={t('workbenchNavigation.sessionSearchHint')} value={search} onChange={event => setSearch(event.target.value)} />{search ? <Button size="xs" variant="ghost" onClick={() => setSearch('')}>{t('patterns.clearSearch')}</Button> : null}</div> : null}
    {query && !matchingIds.size ? <p role="status" className="m-0 text-xs text-ink-secondary">{t('workbenchNavigation.noMatches')}</p> : null}
    <nav aria-label={t('dev.sessions')} className="dev-session-list grid gap-1">
      {visible.map(session => {
        const status = live[session.id];
        return <div key={session.id} className="dev-session-item" data-testid={`dev-session-item-${session.id}`}>
          <Button size="xs" variant={selectedId === session.id ? 'secondary' : 'ghost'} className="dev-session-select" aria-pressed={selectedId === session.id} aria-describedby={!status && !trusted ? trustHintId : undefined} disabled={busy || (!status && !trusted)} onClick={() => props.onResume(session)}>
            <span className="dev-nav-item-label" title={`${session.id}\n${session.workspace}`}>{session.engine === 'codex' ? 'Codex' : 'Claude'} · {session.id.slice(0, 8)}</span>
            <span className="dev-session-meta" title={session.workspace}>{status ? t(`agent.state.${status}`) : session.workspace}</span>
            <span className="dev-session-action">{selectedId === session.id ? `${t('workbenchNavigation.current')} · ` : ''}{status ? t('dev.resume') : t('dev.trustResume')}</span>
          </Button>
        </div>;
      })}
    </nav>
    {sessions.some(session => !live[session.id]) && !trusted ? <div className="dev-session-trust">
      <p id={trustHintId} className="m-0 text-xs text-ink-secondary">{t('dev.error.workspace_untrusted')}</p>
      <Button size="xs" variant="secondary" aria-expanded={setupOpen} aria-controls={setupId} onClick={() => setSetupOpen(true)}>{t('dev.confirmTrust')}</Button>
    </div> : null}
    <details id={setupId} className="dev-disclosure" open={setupOpen} onToggle={event => setSetupOpen(event.currentTarget.open)}>
      <summary className="dev-disclosure-summary">{t('dev.newSession')}</summary>
      <div className="dev-disclosure-body dev-session-setup">
        <p className="m-0 break-all text-xs" data-testid="dev-session-cwd">{t('review.cwd')}: {props.workspacePath ?? t('delivery.unknown')}</p>
        <label className="dev-session-trust flex items-start gap-2 text-xs"><input type="checkbox" data-testid="dev-trust" checked={trusted} onChange={event => props.onTrusted(event.target.checked)} /><span>{t('dev.trustProject')}</span></label>
        <p className="m-0 text-xs text-ink-secondary">{t('dev.trustHint')} {t('dev.resumeNotStarted')}</p>
        <Select label={t('capabilities.newSessionSource')} value={props.selectedCapabilities ? String(props.selectedCapabilities.revision) : 'native'} disabled={busy} onChange={value => props.onCapabilities(value === 'native' ? null : props.projectCapabilities?.revision === Number(value) ? props.projectCapabilities : props.selectedCapabilities)} options={[{ value: 'native', label: t('capabilities.native') }, ...(props.selectedCapabilities ? [{ value: String(props.selectedCapabilities.revision), label: t('capabilities.projectRevision', { revision: props.selectedCapabilities.revision }) }] : []), ...(props.projectCapabilities && props.projectCapabilities.revision !== props.selectedCapabilities?.revision ? [{ value: String(props.projectCapabilities.revision), label: t('capabilities.projectRevision', { revision: props.projectCapabilities.revision }) }] : [])]} />
        {props.capabilitiesLoading ? <p role="status" className="text-xs text-ink-secondary">{t('capabilities.loading')}</p> : null}
        {props.capabilitiesError ? <p role="alert" className="text-xs text-status-error">{props.capabilitiesError}</p> : null}
        {props.selectedCapabilities ? <p className="m-0 break-all text-xs">{t('capabilities.fixedModel', { engine: props.selectedCapabilities.configuration.engine === 'codex' ? 'Codex' : 'Claude', model: props.selectedCapabilities.configuration.provider.source === 'dgos' ? props.selectedCapabilities.configuration.provider.model_id : t('capabilities.native') })}</p> : <>
          <Select label={t('agent.engine')} value={props.engine} onChange={value => props.onEngine(value === 'claude' ? 'claude' : 'codex')} options={[{ value: 'codex', label: t('agent.engine.codex') }, { value: 'claude', label: t('agent.engine.claude') }]} />
          <Input label={t('agent.model')} value={props.model} onChange={event => props.onModel(event.target.value)} />
        </>}
        <p className="m-0 text-xs text-ink-secondary">{t('capabilities.restorePinned')}</p>
        {task && !props.workspacePath ? <p role="alert" className="m-0 text-xs text-status-error">{t('dev.error.invalid_workspace')}</p> : null}
        {task && !trusted ? <p className="m-0 text-xs text-ink-secondary">{t('dev.error.workspace_untrusted')}</p> : null}
        {task ? <Button size="xs" data-testid={`dev-session-new-${props.selectedCapabilities?.configuration.engine ?? props.engine}`} disabled={!canStart} onClick={props.onStart}>{t('dev.newSession')}</Button> : null}
      </div>
    </details>
  </section>;
}

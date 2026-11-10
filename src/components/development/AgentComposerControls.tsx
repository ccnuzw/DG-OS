import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Button, Popover, Slider } from '@dgos/ui';
import { Brain, Bot, Check, ClipboardList, Lightbulb, Paperclip, Settings2, Shield, Zap } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession, type Engine } from '../../stores/agentRunnerStore';
import { agentDraftSettingsKey, agentSettingsKey, agentTurnSettings, setAgentDefaults, type AgentCollaborationMode, type AgentTurnSettings } from '../../stores/agentTurnSettings';
import { useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { useDevelopment } from '../../stores/developmentStore';
import { useDesktopPersistence } from '../../stores/developmentDesktop';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { WorkspaceLink } from '../chat/RuntimeNavigation';
import { agentContextUsage } from './agentContextUsage';
import { AgentContextDetails } from './AgentContextDetails';
import { AgentChatPreferences } from './AgentChatPreferences';
import './agent-composer-controls.css';
import { useNativeManagement } from '../nativeExtensions/NativeManagementContext';

type Panel = 'reasoning' | 'permission' | 'context' | 'more' | 'references';
type RunMode = 'default' | 'accept_edits' | 'plan' | 'deep' | 'full_auto';
const modes = [
  { mode: 'default', Icon: Shield }, { mode: 'accept_edits', Icon: Bot },
  { mode: 'plan', Icon: ClipboardList }, { mode: 'deep', Icon: Lightbulb }, { mode: 'full_auto', Icon: Zap },
] as const;
const effortOrder = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'];
export function AgentComposerControls({ session, projectId, engine = session?.engine, draftId, model, references, referencePicker, more, send, sendHint, disabled = false, visible = true }: { session?: AgentSession; projectId: string | null; engine?: Engine; draftId?: string; model: ReactNode; references?: ReactNode; referencePicker?: ReactNode; more?: ReactNode; send?: ReactNode; sendHint?: ReactNode; disabled?: boolean; visible?: boolean }) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const openNativeManagement = useNativeManagement();
  const taskId = useDevelopment(state => state.selection.project_id === projectId ? state.selection.task_id : null);
  const { generation, pending } = useContextSelection();
  const surfaceActive = useSurfaceActive();
  const active = visible && surfaceActive;
  const key = session ? agentSettingsKey(session) : draftId ? agentDraftSettingsKey(projectId, draftId) : '';
  const proposed = useDevelopmentDrafts(state => state.entries[key]?.value as AgentTurnSettings | undefined);
  useDesktopPersistence(state => session ? state.value[`agent-session:${session.id}`] : undefined);
  const draft = session ? agentTurnSettings(session) : proposed;
  const controls = session?.controls;
  const [panel, setPanel] = useState<{ owner: string; name: Panel } | null>(null);
  const [defaultSaved, setDefaultSaved] = useState(false);
  const owner = JSON.stringify([epoch, generation, projectId, session?.id, session?.generation, draftId, engine, controls, session?.status, session?.pending, disabled, pending, active]);
  const writable = active && !pending && !disabled && session?.status === 'idle' && !session.active_run && !session.pending.length;
  const mounted = useRef(true);
  const latestOwner = useRef(owner);
  latestOwner.current = owner;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setPanel(null); setDefaultSaved(false); }, [owner]);
  const effort = draft?.reasoning_effort !== undefined ? draft.reasoning_effort : controls?.reasoning.current ?? null;
  const permission = draft?.permission_mode ?? controls?.permission.current ?? 'default';
  const collaboration = draft?.collaboration_mode ?? controls?.collaboration?.current ?? 'default';
  const selectedMode: RunMode = collaboration === 'plan' || collaboration === 'deep' ? collaboration : permission === 'read_only' ? 'default' : permission;
  const ModeIcon = modes.find(item => item.mode === selectedMode)!.Icon;
  const usage = agentContextUsage(session);
  const effortOptions: (string | null)[] = [null, ...[...(controls?.reasoning.options ?? [])].sort((a, b) => {
    const ai = effortOrder.indexOf(a), bi = effortOrder.indexOf(b);
    return (ai < 0 ? effortOrder.length : ai) - (bi < 0 ? effortOrder.length : bi);
  })];
  const effortLabel = (value: string | null) => value === null ? t('agentControls.default') : effortOrder.includes(value) ? t(`agentControls.effort.${value}`) : value;
  function current() {
    const runner = useAgentRunner.getState();
    const live = runner.session;
    return mounted.current && latestOwner.current === owner && writable && session && epoch === useDevelopment.getState().epoch && generation === contextGeneration() && runner.selected === session.id && live?.id === session.id && live.generation === session.generation && !runner.busy && !runner.acting && live.status === 'idle' && !live.active_run && !live.pending.length && JSON.stringify(live.controls) === JSON.stringify(controls);
  }
  function update(patch: Partial<AgentTurnSettings>, close = true) {
    if (!current()) return;
    if (patch.reasoning_effort !== undefined && (!controls?.reasoning.available || patch.reasoning_effort !== null && !controls.reasoning.options.includes(patch.reasoning_effort))) return;
    if (patch.permission_mode !== undefined && (!controls?.permission.available || !controls.permission.options.includes(patch.permission_mode))) return;
    if (patch.collaboration_mode !== undefined && (!controls?.collaboration?.available || !controls.collaboration.options.includes(patch.collaboration_mode))) return;
    useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: { ...(state.entries[key]?.value as AgentTurnSettings | undefined), ...patch }, baseline: {} } } }));
    if (close) setPanel(null);
  }
  function modeAvailable(mode: RunMode) {
    if (mode === 'plan' || mode === 'deep') return Boolean(controls?.collaboration?.available && controls.collaboration.options.includes(mode));
    return Boolean(controls?.permission.available && controls.permission.options.includes(mode) && (collaboration === 'default' || controls.collaboration?.available && controls.collaboration.options.includes('default')));
  }
  function chooseMode(mode: RunMode) {
    if (!modeAvailable(mode)) return;
    if (mode === 'plan' || mode === 'deep') update({ collaboration_mode: mode });
    else update({ permission_mode: mode, ...(controls?.collaboration?.available && controls.collaboration.options.includes('default') ? { collaboration_mode: 'default' as AgentCollaborationMode } : {}) });
  }
  const manage = (kind: 'skills' | 'mcp') => {
    if (!active || pending || !engine || !openNativeManagement) return;
    setPanel(null);
    openNativeManagement({ kind, selection: { target: engine === 'claude' ? 'Claude' : 'Codex', scope: projectId ? 'ProjectShared' : 'OsUser', project_id: projectId, task_id: taskId }, sessionId: session?.id });
  };
  function popup(name: Panel, trigger: ReactNode, content: ReactNode, title?: string) {
    return <span className={`agent-control-entry agent-control-entry-${name}`}><Popover label={t(`agentControls.${name}`)} side="top" align={name === 'more' || name === 'references' ? 'start' : 'end'} collisionPadding={12} open={active && !pending && panel?.owner === owner && panel.name === name} onOpenChange={open => setPanel(open ? { owner, name } : null)} onCloseAutoFocus={event => { if (!active || pending || latestOwner.current !== owner) event.preventDefault(); }} trigger={<Button size="xs" variant="ghost" aria-label={t(`agentControls.${name}`)} title={title ?? t(`agentControls.${name}`)} disabled={!active || pending}>{trigger}</Button>} content={<div className={`agent-control-popover agent-control-popover-${name}`}>{content}</div>} /></span>;
  }
  const ratio = usage.used !== null && usage.limit ? Math.min(100, Math.max(0, usage.used / usage.limit * 100)) : null;
  return <div className="agent-composer-controls" data-testid="agent-composer-controls">
    <div className="agent-control-leading">
      {referencePicker}
      {references ? popup('references', <Paperclip size={16} aria-hidden="true" />, references) : null}
      {popup('more', <Settings2 size={17} aria-hidden="true" />, <>
        <AgentChatPreferences projectId={projectId} session={session} draftId={draftId} disabled={!active || pending} />
        <details className="agent-control-management"><summary>{t('agentControls.management')}</summary><nav className="agent-control-more-links" aria-label={t('agentControls.management')}>
          {engine && openNativeManagement ? <><Button size="xs" variant="ghost" onClick={() => manage('skills')}>{t('nav.skills')}</Button><Button size="xs" variant="ghost" onClick={() => manage('mcp')}>{t('nav.mcp')}</Button></> : null}
          <WorkspaceLink to="/settings/providers">{t('capabilities.provider')}</WorkspaceLink><WorkspaceLink to="/settings/cli">{t('agent.engine')}</WorkspaceLink>
        </nav>{more}
        {projectId && session?.configuration?.format === 'provider_bound' ? <Button size="xs" variant="ghost" disabled={!writable} onClick={() => {
          if (!current() || session.configuration?.format !== 'provider_bound') return;
          const binding = session.configuration.snapshot;
          const settings: AgentTurnSettings = { ...draft };
          if (settings.reasoning_effort === undefined && controls?.reasoning.available && !controls.reasoning.reason) settings.reasoning_effort = controls.reasoning.current;
          if (settings.permission_mode === undefined && controls?.permission.available && controls.permission.current && !controls.permission.reason) settings.permission_mode = controls.permission.current;
          if (settings.collaboration_mode === undefined && controls?.collaboration?.available && controls.collaboration.current && !controls.collaboration.reason) settings.collaboration_mode = controls.collaboration.current;
          setAgentDefaults(projectId, { configuration: { engine: session.engine, provider: { provider_id: binding.provider_id, model_id: binding.model_id, provider_revision: binding.provider_revision } }, settings });
          setDefaultSaved(true);
        }}>{t('agentControls.saveProjectDefault')}</Button> : null}
        {defaultSaved ? <p role="status">{t('agentControls.projectDefaultSaved')}</p> : null}</details>
      </>, t('agentControls.settings'))}
    </div>
    <div className="agent-control-trailing">
      {popup('permission', <><ModeIcon size={15} aria-hidden="true" className={selectedMode === 'full_auto' ? 'agent-control-danger' : undefined} /><span>{t(`agentControls.mode.${selectedMode}`)}</span></>, <>
        <h3 className="agent-mode-heading">{t('agentControls.runMode')}</h3>
        <div className="agent-mode-options" role="group" aria-label={t('agentControls.runMode')}>
          {modes.map(({ mode, Icon }) => <Button key={mode} variant="ghost" className={`agent-mode-option ${mode === 'full_auto' ? 'agent-mode-danger' : ''}`} aria-pressed={selectedMode === mode} disabled={!writable || !modeAvailable(mode)} onClick={() => chooseMode(mode)}>
            <Icon size={19} aria-hidden="true" /><span className="agent-mode-copy"><strong>{t(`agentControls.mode.${mode}`)}</strong><small>{t(modeAvailable(mode) ? `agentControls.modeDescription.${mode}` : 'agentControls.modeUnavailable')}</small></span>{selectedMode === mode ? <Check size={17} aria-hidden="true" /> : null}
          </Button>)}
        </div>
      </>)}
      <div className="agent-control-model-reasoning"><div className="agent-control-model">{model}</div>
        {popup('reasoning', <><Brain size={16} aria-hidden="true" />{controls?.reasoning.available ? <span>{effortLabel(effort)}</span> : null}</>, controls?.reasoning.available ? <>
          <div className="agent-reasoning-heading"><span>{t('agentControls.reasoningShort')}</span><strong>{effortLabel(effort)}</strong></div>
          <Slider label={t('agentControls.reasoning')} hideLabel min={0} max={effortOptions.length - 1} step={1} value={Math.max(0, effortOptions.indexOf(effort))} disabled={!writable} onValueChange={index => update({ reasoning_effort: effortOptions[index] }, false)} />
          <div className="agent-reasoning-ticks" role="group" aria-label={t('agentControls.reasoning')}>{effortOptions.map(value => <Button size="xs" variant="ghost" key={value ?? 'default'} disabled={!writable} aria-pressed={effort === value} onClick={() => update({ reasoning_effort: value }, false)}>{effortLabel(value)}</Button>)}</div>
        </> : <p role="status">{t(!session || !controls ? 'agentControls.loadingCapabilities' : 'agentControls.reasoningUnavailable')}</p>, `${t('agentControls.reasoning')}：${controls?.reasoning.available ? effortLabel(effort) : t('agentControls.unavailable')}`)}
      </div>
      <span className="agent-control-divider" aria-hidden="true" />
      {popup('context', <svg className="agent-usage-ring" width="20" height="20" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" className="agent-usage-ring-track" />{ratio !== null ? <circle cx="12" cy="12" r="9" className="agent-usage-ring-value" pathLength="100" strokeDasharray={`${ratio} 100`} /> : null}</svg>, <AgentContextDetails usage={usage} session={session} disabled={!writable} />)}
      {send ? <div className="agent-control-send">{sendHint ? <span className="agent-control-send-hint" id="agent-send-hint">{sendHint}</span> : null}{send}</div> : null}
    </div>
  </div>;
}

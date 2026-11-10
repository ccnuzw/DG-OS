import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Button, Popover } from '@dgos/ui';
import { Brain, Gauge, Paperclip, Settings2, Shield } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession, type Engine } from '../../stores/agentRunnerStore';
import { agentDraftSettingsKey, agentSettingsKey, setAgentDefaults, type AgentPermissionMode, type AgentTurnSettings } from '../../stores/agentTurnSettings';
import { useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { WorkspaceLink } from '../chat/RuntimeNavigation';
import { agentContextUsage } from './agentContextUsage';
import './agent-composer-controls.css';
import { AgentContextDetails } from './AgentContextDetails';

type Panel = 'reasoning' | 'permission' | 'context' | 'more' | 'references';
export function AgentComposerControls({ session, projectId, engine = session?.engine, draftId, model, references, referencePicker, more, send, sendHint, disabled = false, visible = true }: { session?: AgentSession; projectId: string | null; engine?: Engine; draftId?: string; model: ReactNode; references?: ReactNode; referencePicker?: ReactNode; more?: ReactNode; send?: ReactNode; sendHint?: ReactNode; disabled?: boolean; visible?: boolean }) {
  const { t, locale } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const surfaceActive = useSurfaceActive();
  const active = visible && surfaceActive;
  const key = session ? agentSettingsKey(session) : draftId ? agentDraftSettingsKey(projectId, draftId) : '';
  const draft = useDevelopmentDrafts(state => state.entries[key]?.value as AgentTurnSettings | undefined);
  const controls = session?.controls;
  const [panel, setPanel] = useState<{ owner: string; name: Panel } | null>(null);
  const [expansion, setExpansion] = useState<{ owner: string; mode: AgentPermissionMode } | null>(null);
  const [defaultSaved, setDefaultSaved] = useState(false);
  const owner = JSON.stringify([epoch, generation, projectId, session?.id, session?.generation, draftId, engine, controls, session?.status, session?.pending, disabled, pending, active]);
  const writable = active && !pending && !disabled && session?.status === 'idle' && !session.active_run && !session.pending.length;
  const mounted = useRef(true);
  const latestOwner = useRef(owner);
  latestOwner.current = owner;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setExpansion(null); setPanel(null); }, [owner]);
  useEffect(() => { setDefaultSaved(false); }, [owner]);
  const settings: Required<Pick<AgentTurnSettings, 'reasoning_effort' | 'permission_mode'>> = { reasoning_effort: draft?.reasoning_effort !== undefined ? draft.reasoning_effort : controls?.reasoning.current ?? null, permission_mode: draft?.permission_mode ?? controls?.permission.current ?? 'default' };
  const awaiting = Boolean(draft && (!controls || draft.reasoning_effort !== undefined && (draft.reasoning_effort !== controls.reasoning.current || Boolean(controls.reasoning.reason)) || draft.permission_mode !== undefined && (draft.permission_mode !== controls.permission.current || Boolean(controls.permission.reason)) || draft.collaboration_mode !== undefined && (draft.collaboration_mode !== controls.collaboration?.current || Boolean(controls.collaboration?.reason))));
  const usage = agentContextUsage(session);
  const number = (value: number) => value.toLocaleString(locale);
  function current() {
    const runner = useAgentRunner.getState();
    const live = runner.session;
    return mounted.current && latestOwner.current === owner && writable && session && epoch === useDevelopment.getState().epoch && generation === contextGeneration() && runner.selected === session.id && live?.id === session.id && live.generation === session.generation && !runner.busy && !runner.acting && live.status === 'idle' && !live.active_run && !live.pending.length && JSON.stringify(live.controls) === JSON.stringify(controls);
  }
  function update(patch: Partial<AgentTurnSettings>) {
    if (!current()) return;
    if (patch.reasoning_effort !== undefined && (!controls?.reasoning.available || patch.reasoning_effort !== null && !controls.reasoning.options.includes(patch.reasoning_effort))) return;
    if (patch.permission_mode !== undefined && (!controls?.permission.available || !controls.permission.options.includes(patch.permission_mode))) return;
    if (patch.collaboration_mode !== undefined && (!controls?.collaboration?.available || !controls.collaboration.options.includes(patch.collaboration_mode))) return;
    useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key]: { kind: 'conversation', value: { ...(state.entries[key]?.value as AgentTurnSettings | undefined), ...patch }, baseline: {} } } }));
    setExpansion(null); setPanel(null);
  }
  function choosePermission(mode: AgentPermissionMode) {
    if (!current() || !controls?.permission.available || !controls.permission.options.includes(mode)) return;
    const previous = controls.permission.current;
    if (mode !== 'read_only' && (mode !== previous || settings.permission_mode === 'read_only')) setExpansion({ owner, mode });
    else update({ permission_mode: mode });
  }
  const availability = !session ? t('agentControls.beforeStart') : !controls ? t('agentControls.unavailable') : t('agentControls.nextTurn');
  const capabilityReason = (reason: string) => {
    const key = `agentControls.reason.${reason}`;
    const translated = t(key);
    return translated === key ? t('agentControls.reason.unavailable') : translated;
  };
  const permissionName = (mode: AgentPermissionMode | null | undefined) => t(mode ? `agentControls.permission.${mode}` : 'agentControls.unknown');
  const entryState = { capabilityEntry: { runtime: 'native', selection: { target: engine === 'claude' ? 'Claude' : 'Codex', scope: projectId ? 'ProjectShared' : 'OsUser', project_id: projectId }, sessionId: session?.id, returnTabId: useDevelopment(state => state.activeTabId), epoch } };
  const management = engine ? <><WorkspaceLink to="/skills" state={entryState}>{t('nav.skills')}</WorkspaceLink><WorkspaceLink to="/mcp" state={entryState}>{t('nav.mcp')}</WorkspaceLink></> : <p>{t('agentControls.chooseModel')}</p>;
  function popup(name: Panel, trigger: ReactNode, content: ReactNode, className?: string) {
    return <span className={className}><Popover label={t(`agentControls.${name}`)} side="top" align="start" open={active && !pending && panel?.owner === owner && panel.name === name} onOpenChange={open => { setPanel(open ? { owner, name } : null); setExpansion(null); }} onCloseAutoFocus={event => { if (!active || pending || panel?.owner !== owner) event.preventDefault(); }} trigger={<Button size="xs" variant="ghost" aria-label={t(`agentControls.${name}`)} disabled={!active || pending}>{trigger}</Button>} content={<div className="agent-control-popover">{content}</div>} /></span>;
  }
  const contextLabel = usage.used === null ? t('agentControls.unavailableShort') : usage.limit ? `≈${Math.round(usage.used / usage.limit * 100)}%` : `≈${number(usage.used)}`;
  return <div className="agent-composer-controls" data-testid="agent-composer-controls">
    <div className="agent-control-leading">
      {referencePicker}
      {references ? popup('references', <><Paperclip size={14} aria-hidden="true" /><span>{t('agentControls.references')}</span></>, <><h3>{t('agentControls.references')}</h3>{references}</>) : null}
      {popup('more', <><Settings2 size={15} aria-hidden="true" /><span className="agent-control-settings-label">{t('agentControls.settings')}</span></>, <>
        <nav className="agent-control-more-links" aria-label={t('agentControls.management')}>{management}<WorkspaceLink to="/settings/providers">{t('capabilities.provider')}</WorkspaceLink><WorkspaceLink to="/settings/cli">{t('agent.engine')}</WorkspaceLink></nav>
        {more}
        {projectId && session?.configuration?.format === 'provider_bound' ? <Button size="xs" variant="secondary" disabled={!writable} onClick={() => {
          if (!current() || session.configuration?.format !== 'provider_bound') return;
          const binding = session.configuration.snapshot;
          const settings: AgentTurnSettings = { ...draft };
          if (settings.reasoning_effort === undefined && controls?.reasoning.available && !controls.reasoning.reason) settings.reasoning_effort = controls.reasoning.current;
          if (settings.permission_mode === undefined && controls?.permission.available && controls.permission.current && !controls.permission.reason) settings.permission_mode = controls.permission.current;
          if (settings.collaboration_mode === undefined && controls?.collaboration?.available && controls.collaboration.current && !controls.collaboration.reason) settings.collaboration_mode = controls.collaboration.current;
          setAgentDefaults(projectId, { configuration: { engine: session.engine, provider: { provider_id: binding.provider_id, model_id: binding.model_id, provider_revision: binding.provider_revision } }, settings });
          setDefaultSaved(true);
        }}>{t('agentControls.saveProjectDefault')}</Button> : null}
        {defaultSaved ? <p role="status">{t('agentControls.projectDefaultSaved')}</p> : null}
        <p>{t('agentControls.managementBoundary')}</p><p>{t('workbenchReferences.sendBoundary')}</p>
      </>)}
    </div>
    <div className="agent-control-trailing">
      {popup('permission', <><Shield size={14} aria-hidden="true" /><span>{permissionName(draft?.permission_mode ?? controls?.permission.current)}</span></>, <>
        <h3>{t('agentControls.taskMode')}</h3><p>{availability}</p>
        {controls?.collaboration?.available ? <div className="agent-control-options" role="group" aria-label={t('agentControls.taskMode')}>
          {controls.collaboration.options.map(mode => <Button key={mode} size="xs" variant={(draft?.collaboration_mode ?? controls.collaboration?.current) === mode ? 'secondary' : 'ghost'} aria-pressed={(draft?.collaboration_mode ?? controls.collaboration?.current) === mode} disabled={!writable} onClick={() => update({ collaboration_mode: mode })}>{t(`codexInteractions.${mode}`)}</Button>)}
        </div> : <p>{t('agentControls.taskModeUnavailable')}</p>}
        <p>{t('agentControls.planBoundary')}</p>
        <p>{t('agentControls.applied')}: {controls?.collaboration?.current ? t(`codexInteractions.${controls.collaboration.current}`) : t('agentControls.unknown')}</p>
        <h3 className="agent-control-group-heading">{t('agentControls.executionPermission')}</h3><p>{t('agentControls.permissionBoundary')}</p>
        {controls?.permission.available ? <div className="agent-control-options" role="group" aria-label={t('agentControls.permission')}>
          {controls.permission.options.map(mode => <Button key={mode} size="xs" variant={settings.permission_mode === mode ? 'secondary' : 'ghost'} aria-pressed={settings.permission_mode === mode} disabled={!writable} onClick={() => choosePermission(mode)}>{permissionName(mode)}</Button>)}
        </div> : <p>{t('agentControls.permissionUnavailable')}</p>}
        <p>{t(`agentControls.scope.${engine ?? 'unknown'}.${settings.permission_mode}`)}</p>
        <p>{t('agentControls.applied')}: {permissionName(controls?.permission.current)}</p>
        {expansion?.owner === owner ? <section className="agent-control-confirm" role="group" aria-label={t('agentControls.expandTitle')}><strong>{t('agentControls.expandTitle')}</strong><p>{t(`agentControls.scope.${engine ?? 'unknown'}.${expansion.mode}`)}</p><p>{t('agentControls.noPendingApproval')}</p><Button size="xs" disabled={!writable} onClick={() => update({ permission_mode: expansion.mode })}>{t('agentControls.confirmExpansion')}</Button><Button size="xs" variant="ghost" onClick={() => setExpansion(null)}>{t('common.cancel')}</Button></section> : null}
        {controls?.permission.reason ? <p>{t('agentControls.capabilityReason')}: {capabilityReason(controls.permission.reason)}</p> : null}
        {controls?.permission.source ? <details><summary>{t('agentControls.source')}</summary><code>{controls.permission.source}</code></details> : null}
      </>)}
      <div className="agent-control-model">{model}</div>
      {popup('reasoning', <><Brain size={14} aria-hidden="true" /><span>{t('agentControls.reasoning')} · {settings.reasoning_effort ?? t(controls?.reasoning.reason ? 'agentControls.unknown' : 'agentControls.default')}</span></>, <>
        <h3>{t('agentControls.reasoning')}</h3><p>{availability}</p>
        {controls?.reasoning.available ? <div className="agent-control-options" role="group" aria-label={t('agentControls.reasoning')}>
          {[null, ...controls.reasoning.options].map(value => <Button key={value ?? 'native-default'} size="xs" variant={settings.reasoning_effort === value ? 'secondary' : 'ghost'} aria-pressed={settings.reasoning_effort === value} disabled={!writable} onClick={() => update({ reasoning_effort: value })}>{value ?? t('agentControls.default')}</Button>)}
        </div> : <p>{t('agentControls.reasoningUnavailable')}</p>}
        <p>{t('agentControls.applied')}: {controls?.reasoning.current ?? t(controls?.reasoning.reason || !controls ? 'agentControls.unknown' : 'agentControls.default')}</p>
        {controls?.reasoning.reason ? <p>{t('agentControls.capabilityReason')}: {capabilityReason(controls.reasoning.reason)}</p> : null}
        {controls?.reasoning.source ? <details><summary>{t('agentControls.source')}</summary><code>{controls.reasoning.source}</code></details> : null}
      </>)}
      {popup('context', <><Gauge size={14} aria-hidden="true" /><span>{t('agentControls.context')} · {contextLabel}</span></>, <>
        <AgentContextDetails usage={usage} />
      </>)}
      {send ? <div className="agent-control-send">{sendHint ? <span className="agent-control-send-hint" id="agent-send-hint">{sendHint}</span> : null}{send}</div> : null}
    </div>
    {awaiting ? <p className="agent-control-intent" role="status">{t('agentControls.pendingSettings')}</p> : null}
  </div>;
}

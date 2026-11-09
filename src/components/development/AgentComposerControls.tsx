import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Button, Popover } from '@dgos/ui';
import { Brain, Gauge, MoreHorizontal, Paperclip, Shield } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession, type Engine } from '../../stores/agentRunnerStore';
import { agentSettingsKey, type AgentPermissionMode, type AgentTurnSettings } from '../../stores/agentTurnSettings';
import { useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { useDevelopment } from '../../stores/developmentStore';
import { contextGeneration, useContextSelection } from '../../stores/developmentSelection';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { WorkspaceLink } from '../chat/RuntimeNavigation';
import { agentContextUsage } from './agentContextUsage';
import './agent-composer-controls.css';

type Panel = 'reasoning' | 'permission' | 'context' | 'more' | 'references';
export function AgentComposerControls({ session, projectId, engine = session?.engine, draftId, model, references, referencePicker, more, disabled = false, visible = true }: { session?: AgentSession; projectId: string | null; engine?: Engine; draftId?: string; model: ReactNode; references?: ReactNode; referencePicker?: ReactNode; more?: ReactNode; disabled?: boolean; visible?: boolean }) {
  const { t, locale } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const { generation, pending } = useContextSelection();
  const surfaceActive = useSurfaceActive();
  const active = visible && surfaceActive;
  const key = session ? agentSettingsKey(session) : '';
  const draft = useDevelopmentDrafts(state => state.entries[key]?.value as AgentTurnSettings | undefined);
  const controls = session?.controls;
  const [panel, setPanel] = useState<{ owner: string; name: Panel } | null>(null);
  const [expansion, setExpansion] = useState<{ owner: string; mode: AgentPermissionMode } | null>(null);
  const owner = JSON.stringify([epoch, generation, projectId, session?.id, session?.generation, draftId, engine, controls, session?.status, session?.pending, disabled, pending, active]);
  const writable = active && !pending && !disabled && session?.status === 'idle' && !session.active_run && !session.pending.length;
  const mounted = useRef(true);
  const latestOwner = useRef(owner);
  latestOwner.current = owner;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setExpansion(null); setPanel(null); }, [owner]);
  const settings: Required<AgentTurnSettings> = { reasoning_effort: draft?.reasoning_effort !== undefined ? draft.reasoning_effort : controls?.reasoning.current ?? null, permission_mode: draft?.permission_mode ?? controls?.permission.current ?? 'default' };
  const awaiting = Boolean(draft && (!controls || draft.reasoning_effort !== undefined && (draft.reasoning_effort !== controls.reasoning.current || Boolean(controls.reasoning.reason)) || draft.permission_mode !== undefined && (draft.permission_mode !== controls.permission.current || Boolean(controls.permission.reason))));
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
  const permissionName = (mode: AgentPermissionMode | null | undefined) => t(mode ? `agentControls.permission.${mode}` : 'agentControls.unknown');
  const entryState = { capabilityEntry: { runtime: 'native', selection: { target: engine === 'claude' ? 'Claude' : 'Codex', scope: projectId ? 'ProjectShared' : 'OsUser', project_id: projectId }, sessionId: session?.id, returnTabId: useDevelopment(state => state.activeTabId), epoch } };
  const management = engine ? <><WorkspaceLink to="/skills" state={entryState}>{t('nav.skills')}</WorkspaceLink><WorkspaceLink to="/mcp" state={entryState}>{t('nav.mcp')}</WorkspaceLink></> : <p>{t('agentControls.chooseModel')}</p>;
  function popup(name: Panel, trigger: ReactNode, content: ReactNode, className?: string) {
    return <span className={className}><Popover label={t(`agentControls.${name}`)} side="top" align="start" open={active && !pending && panel?.owner === owner && panel.name === name} onOpenChange={open => { setPanel(open ? { owner, name } : null); setExpansion(null); }} onCloseAutoFocus={event => { if (!active || pending || panel?.owner !== owner) event.preventDefault(); }} trigger={<Button size="xs" variant="ghost" aria-label={t(`agentControls.${name}`)} disabled={!active || pending}>{trigger}</Button>} content={<div className="agent-control-popover">{content}</div>} /></span>;
  }
  const contextLabel = usage.used === null ? t('agentControls.unavailableShort') : usage.limit ? `≈${Math.round(usage.used / usage.limit * 100)}%` : `≈${number(usage.used)}`;
  return <div className="agent-composer-controls" data-testid="agent-composer-controls">
    <div className="agent-control-primary">
      <div className="agent-control-model">{model}</div>
      {popup('reasoning', <><Brain size={14} aria-hidden="true" /><span>{t('agentControls.reasoning')} · {settings.reasoning_effort ?? t(controls?.reasoning.reason ? 'agentControls.unknown' : 'agentControls.default')}</span></>, <>
        <h3>{t('agentControls.reasoning')}</h3><p>{availability}</p>
        {controls?.reasoning.available ? <div className="agent-control-options" role="group" aria-label={t('agentControls.reasoning')}>
          {[null, ...controls.reasoning.options].map(value => <Button key={value ?? 'native-default'} size="xs" variant={settings.reasoning_effort === value ? 'secondary' : 'ghost'} aria-pressed={settings.reasoning_effort === value} disabled={!writable} onClick={() => update({ reasoning_effort: value })}>{value ?? t('agentControls.default')}</Button>)}
        </div> : <p>{t('agentControls.reasoningUnavailable')}</p>}
        <p>{t('agentControls.applied')}: {controls?.reasoning.current ?? t(controls?.reasoning.reason || !controls ? 'agentControls.unknown' : 'agentControls.default')}</p>
        {controls?.reasoning.reason ? <p>{t('agentControls.capabilityReason')}: {controls.reasoning.reason}</p> : null}
        {controls?.reasoning.source ? <details><summary>{t('agentControls.source')}</summary><code>{controls.reasoning.source}</code></details> : null}
      </>)}
      {popup('permission', <><Shield size={14} aria-hidden="true" /><span>{permissionName(draft?.permission_mode ?? controls?.permission.current)}</span></>, <>
        <h3>{t('agentControls.permission')}</h3><p>{availability}</p><p>{t('agentControls.permissionBoundary')}</p>
        {controls?.permission.available ? <div className="agent-control-options" role="group" aria-label={t('agentControls.permission')}>
          {controls.permission.options.map(mode => <Button key={mode} size="xs" variant={settings.permission_mode === mode ? 'secondary' : 'ghost'} aria-pressed={settings.permission_mode === mode} disabled={!writable} onClick={() => choosePermission(mode)}>{permissionName(mode)}</Button>)}
        </div> : <p>{t('agentControls.permissionUnavailable')}</p>}
        <p>{t(`agentControls.scope.${engine ?? 'unknown'}.${settings.permission_mode}`)}</p>
        <p>{t('agentControls.applied')}: {permissionName(controls?.permission.current)}</p>
        {expansion?.owner === owner ? <section className="agent-control-confirm" role="group" aria-label={t('agentControls.expandTitle')}><strong>{t('agentControls.expandTitle')}</strong><p>{t(`agentControls.scope.${engine ?? 'unknown'}.${expansion.mode}`)}</p><p>{t('agentControls.noPendingApproval')}</p><Button size="xs" disabled={!writable} onClick={() => update({ permission_mode: expansion.mode })}>{t('agentControls.confirmExpansion')}</Button><Button size="xs" variant="ghost" onClick={() => setExpansion(null)}>{t('common.cancel')}</Button></section> : null}
        {controls?.permission.reason ? <p>{t('agentControls.capabilityReason')}: {controls.permission.reason}</p> : null}
        {controls?.permission.source ? <details><summary>{t('agentControls.source')}</summary><code>{controls.permission.source}</code></details> : null}
      </>)}
      {popup('context', <><Gauge size={14} aria-hidden="true" /><span>{t('agentControls.context')} · {contextLabel}</span></>, <>
        <h3>{t('agentControls.context')}</h3><dl><dt>{t('agentControls.used')}</dt><dd>{usage.used === null ? t('agentControls.unavailable') : `${number(usage.used)} ${t('agentControls.estimated')}`}</dd><dt>{t('agentControls.limit')}</dt><dd>{usage.limit === null ? t('agentControls.unavailable') : number(usage.limit)}</dd><dt>{t('agentControls.source')}</dt><dd>{usage.source || t('agentControls.noNativeData')}</dd></dl><p>{t('agentControls.contextExplanation')}</p>{session && ['running', 'waiting'].includes(session.status) ? <p>{t('agentControls.previousUsage')}</p> : null}
      </>)}
    </div>
    <div className="agent-control-secondary">
      {referencePicker}
      {references ? popup('references', <><Paperclip size={14} aria-hidden="true" /><span>{t('agentControls.references')}</span></>, <><h3>{t('agentControls.references')}</h3>{references}</>) : null}
      <nav className="agent-control-management" aria-label={t('agentControls.management')}>{management}</nav>
      {popup('more', <MoreHorizontal size={16} aria-hidden="true" />, <>
        <nav className="agent-control-more-links" aria-label={t('agentControls.management')}>{management}<WorkspaceLink to="/settings/providers">{t('capabilities.provider')}</WorkspaceLink><WorkspaceLink to="/settings/cli">{t('agent.engine')}</WorkspaceLink></nav>
        {more}
        <p>{t('agentControls.managementBoundary')}</p><p>{t('workbenchReferences.sendBoundary')}</p>
      </>)}
    </div>
    {awaiting ? <p className="agent-control-intent" role="status">{t('agentControls.pendingSettings')}</p> : null}
  </div>;
}

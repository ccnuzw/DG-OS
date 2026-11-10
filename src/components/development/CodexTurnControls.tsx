import { useEffect, useRef, useState } from 'react';
import { Button } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useAgentRunner, type AgentSession } from '../../stores/agentRunnerStore';
import { agentSettingsKey, type AgentCollaborationMode, type AgentTurnSettings } from '../../stores/agentTurnSettings';
import { developmentDraftKey, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { useDevelopment } from '../../stores/developmentStore';
import { pendingAgentDraftKeys } from './agentPendingRequests';

type SteeringDraft = { requestId: string; runId: string; text: string; generation?: string };
export function CodexTurnControls({ session, projectId, prompt, setDraft, currentOwner, disabled = false, hideMode = false }: {
  session: AgentSession; projectId: string | null; prompt: string;
  setDraft: (id: string, value: string, expected?: string) => void;
  currentOwner: () => boolean; disabled?: boolean; hideMode?: boolean;
}) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const runnerEpoch = useAgentRunner(state => state.epoch);
  const acting = useAgentRunner(state => state.acting);
  const settingsKey = agentSettingsKey(session);
  const entries = useDevelopmentDrafts(state => state.entries);
  const pendingKeys = pendingAgentDraftKeys(entries, epoch, projectId, session.id, 'pending-steer');
  const pendingKey = pendingKeys[0] ?? developmentDraftKey('conversation', projectId, session.id, session.generation, 'pending-steer');
  const settings = entries[settingsKey]?.value as AgentTurnSettings | undefined;
  const pending = entries[pendingKey]?.value as SteeringDraft | undefined;
  const [sending, setSending] = useState(false);
  const [failure, setFailure] = useState(false);
  const inFlight = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const mode = session.controls?.collaboration;
  const selected = settings?.collaboration_mode ?? mode?.current;
  const writable = !disabled && !acting && session.status === 'idle' && !session.active_run && !session.pending.length && !session.persistence_error;
  const running = Boolean(session.active_run) && ['running', 'waiting'].includes(session.status);
  const sameAccount = () => epoch === useDevelopment.getState().epoch && runnerEpoch === useAgentRunner.getState().epoch;
  function clear(capture: SteeringDraft, applied: boolean) {
    if (!sameAccount()) return;
    if (applied) setDraft(session.id, '', capture.text);
    useDevelopmentDrafts.setState(state => {
      const keys = pendingAgentDraftKeys(state.entries, epoch, projectId, session.id, 'pending-steer').filter(key => {
        const pending = state.entries[key].value as SteeringDraft;
        return pending.requestId === capture.requestId && pending.runId === capture.runId;
      });
      if (!keys.length) return state;
      const entries = { ...state.entries };
      for (const key of keys) delete entries[key];
      return { entries };
    });
  }
  // Only exact positive evidence acknowledges a pending request. Terminal/absence is not rejection.
  useEffect(() => {
    if (pending && session.events.some(event => event.run === pending.runId && event.kind === 'userSteering' && event.data !== null && typeof event.data === 'object' && !Array.isArray(event.data) && event.data.requestId === pending.requestId)) clear(pending, true);
  }, [session.events, pending, epoch, runnerEpoch]);
  function choose(modeValue: AgentCollaborationMode) {
    const live = useAgentRunner.getState().session;
    if (!writable || !currentOwner() || !sameAccount() || live?.generation !== session.generation || live.status !== 'idle' || live.active_run || !live.controls?.collaboration?.available || !live.controls.collaboration.options.includes(modeValue)) return;
    useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [settingsKey]: { kind: 'conversation', value: { ...(state.entries[settingsKey]?.value as AgentTurnSettings | undefined), collaboration_mode: modeValue }, baseline: {} } } }));
  }
  async function steer() {
    if (inFlight.current || pending || disabled || !running || !prompt.trim() || !session.controls?.steering || session.persistence_error || !currentOwner() || !sameAccount()) return;
    const entries = useDevelopmentDrafts.getState().entries;
    if (pendingAgentDraftKeys(entries, epoch, projectId, session.id, 'pending-send').length || pendingAgentDraftKeys(entries, epoch, projectId, session.id, 'pending-steer').length) return;
    const live = useAgentRunner.getState().session;
    if (live?.generation !== session.generation || live.active_run !== session.active_run) return;
    const capture: SteeringDraft = { requestId: crypto.randomUUID(), runId: session.active_run!, text: prompt, generation: session.generation };
    inFlight.current = true; setSending(true); setFailure(false);
    useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [pendingKey]: { kind: 'conversation', value: capture, baseline: null } } }));
    try {
      let code: string | undefined;
      const applied = await useAgentRunner.getState().action('steer_agent_turn', { input: { session_id: session.id, generation: session.generation, run_id: capture.runId, request_id: capture.requestId, text: capture.text } }, value => { code = value; });
      if (applied) clear(capture, true);
      else if (code && code !== 'unknown') { clear(capture, false); if (mounted.current && sameAccount()) setFailure(true); }
    } finally { inFlight.current = false; if (mounted.current) setSending(false); }
  }
  if (session.engine !== 'codex' || hideMode && !running && !pending && !failure) return null;
  return <section aria-label={t('codexInteractions.title')} className="agent-dialogue-native">
    {!hideMode ? <details><summary>{t('codexInteractions.mode')} · {selected ? t(`codexInteractions.${selected}`) : t('codexInteractions.unavailable')}</summary>
      <p>{t('codexInteractions.modeBoundary')}</p>
      {mode?.available ? <div className="dev-review-toolbar" role="group" aria-label={t('codexInteractions.mode')}>
        {mode.options.map(value => <Button key={value} size="xs" variant={selected === value ? 'secondary' : 'ghost'} aria-pressed={selected === value} disabled={!writable} onClick={() => choose(value)}>{t(`codexInteractions.${value}`)}</Button>)}
      </div> : <p>{t('codexInteractions.unavailable')}</p>}
      <p>{t('codexInteractions.applied')}: {mode?.current ? t(`codexInteractions.${mode.current}`) : t('codexInteractions.unavailable')}</p>
      {settings?.collaboration_mode && (settings.collaboration_mode !== mode?.current || mode?.reason) ? <p role="status">{t('codexInteractions.nextTurn')}</p> : null}
    </details> : null}
    {running ? <div className="dev-review-toolbar"><Button size="xs" variant="secondary" disabled={disabled || sending || Boolean(pending) || !prompt.trim() || !session.controls?.steering || Boolean(session.persistence_error)} onClick={() => void steer()}>{t(sending ? 'codexInteractions.sending' : 'codexInteractions.steer')}</Button><small>{t(session.controls?.steering ? 'codexInteractions.steerBoundary' : 'codexInteractions.steerUnavailable')}</small></div> : null}
    {pending && !sending ? <div role="status"><p>{t('codexInteractions.unknown')}</p><Button size="xs" variant="secondary" disabled={disabled} onClick={() => { if (currentOwner() && sameAccount()) void useAgentRunner.getState().attach(session.id); }}>{t('codexInteractions.recheck')}</Button></div> : null}
    {failure ? <p role="alert">{t('codexInteractions.rejected')}</p> : null}
  </section>;
}

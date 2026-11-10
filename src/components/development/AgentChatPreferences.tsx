import { useEffect, useRef, useState } from 'react';
import { Link } from 'react-router-dom';
import { Button, Select, useAppearance } from '@dgos/ui';
import { ArrowUpRight } from 'lucide-react';
import { useLocale } from '../../i18n';
import { appearanceValues, type Appearance } from '../../ui/appearance/model';
import { saveAgentChatPreferences, useAgentChatPreferences, type AgentChatPreferenceScope, type AgentChatPreferencesValue } from '../../stores/agentChatPreferences';
import { useDevelopment } from '../../stores/developmentStore';
import { useDesktopPersistence } from '../../stores/developmentDesktop';
import './agent-chat-preferences.css';

export function AgentChatPreferences({ projectId, session, draftId, disabled = false, onOpenSettings }: AgentChatPreferenceScope & { disabled?: boolean; onOpenSettings?: () => void }) {
  const { t } = useLocale();
  const scope = { projectId, session, draftId };
  const preferences = useAgentChatPreferences(scope);
  const { state: appearance, controller } = useAppearance();
  const epoch = useDevelopment(state => state.epoch);
  const loaded = useDesktopPersistence(state => state.loaded);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(false);
  const lock = useRef(false);
  const identity = `${epoch}:${projectId}:${session?.id ?? draftId ?? projectId}`;
  const current = useRef<string | null>(identity); current.current = identity;
  useEffect(() => { current.current = identity; setSaving(false); setError(false); lock.current = false; return () => { current.current = null; }; }, [identity]);
  async function change(patch: Partial<AgentChatPreferencesValue>) {
    if (lock.current || disabled || !loaded) return;
    lock.current = true; setSaving(true); setError(false);
    const owner = identity;
    const ok = await saveAgentChatPreferences(scope, patch);
    if (current.current === owner) { lock.current = false; setSaving(false); setError(!ok); }
  }
  async function theme(mode: string) {
    if (!appearance.confirmed || appearance.phase !== 'applied' || lock.current) return;
    lock.current = true; setSaving(true); setError(false);
    const owner = identity;
    controller.preview({ ...appearanceValues(appearance.confirmed), mode: mode as Appearance['mode'] });
    await controller.apply();
    const failed = controller.getSnapshot().phase !== 'applied';
    if (failed && controller.getSnapshot().phase !== 'unconfirmed') controller.cancel();
    if (current.current === owner) { lock.current = false; setSaving(false); setError(failed); }
  }
  const blocked = disabled || saving || !loaded;
  const nextTurnLocked = Boolean(session && (session.active_run || ['running', 'waiting'].includes(session.status)));
  const options = (prefix: string, values: string[]) => values.map(value => ({ value, label: t(`agentPreferences.${prefix}.${value}`) }));
  const toggles = ['next_step_suggestions', 'follow_edits', 'stream_response', 'code_highlighting'] as const;
  return <section className="agent-chat-preferences" aria-label={t('agentPreferences.title')} aria-busy={saving}>
    <div className="agent-chat-preference-select"><Select label={t('agentPreferences.language')} value={preferences.language} disabled={blocked || nextTurnLocked} onChange={value => void change({ language: value as AgentChatPreferencesValue['language'] })} options={options('language', ['zh', 'en', 'follow_input'])} /></div>
    <div className="agent-chat-preference-select"><Select label={t('agentPreferences.theme')} value={appearance.confirmed?.mode ?? appearance.draft.mode} disabled={blocked || appearance.phase !== 'applied'} onChange={value => void theme(value)} options={options('theme', ['light', 'dark', 'auto'])} /></div>
    <div className="agent-chat-preference-select"><Select label={t('agentPreferences.brevity')} value={preferences.brevity} disabled={blocked || nextTurnLocked} onChange={value => void change({ brevity: value as AgentChatPreferencesValue['brevity'] })} options={options('brevity', ['concise', 'standard', 'detailed'])} /></div>
    <div className="agent-chat-preference-toggles">{toggles.map(key => <label className="agent-chat-preference-toggle" key={key}><span>{t(`agentPreferences.${key}`)}</span><input type="checkbox" role="switch" checked={preferences[key]} disabled={blocked || key === 'next_step_suggestions' && nextTurnLocked} onChange={event => void change({ [key]: event.target.checked })} /><span className="agent-chat-preference-track" aria-hidden="true" /></label>)}</div>
    {nextTurnLocked ? <p className="agent-chat-preferences-hint">{t('agentPreferences.nextTurnLocked')}</p> : null}
    {error ? <p role="alert" className="agent-chat-preferences-error">{t('agentPreferences.saveFailed')}</p> : null}
    {appearance.phase === 'unconfirmed' ? <Button size="xs" variant="ghost" disabled={saving} onClick={() => void controller.verify()}>{t('agentPreferences.verifyTheme')}</Button> : null}
    {onOpenSettings ? <Button variant="ghost" className="agent-chat-preferences-settings" onClick={onOpenSettings}>{t('agentPreferences.fullSettings')}<ArrowUpRight size={15} aria-hidden="true" /></Button> : <Link className="agent-chat-preferences-settings" to="/settings/appearance">{t('agentPreferences.fullSettings')}<ArrowUpRight size={15} aria-hidden="true" /></Link>}
  </section>;
}

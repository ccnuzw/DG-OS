import { useEffect, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { readSessionConfiguration, type SessionConfigurationView } from '../../stores/developmentCapabilities';
import { resolveAgentDefaults } from '../../stores/agentTurnSettings';
import { useDevelopment, type DevelopmentSession } from '../../stores/developmentStore';
import { WorkspaceLink } from '../chat/RuntimeNavigation';

export function SavedConversationComposer({ session, value, onChange, busy, authorized, onAuthorize, onPrepare, onSend, onHandoff }: {
  session: DevelopmentSession;
  value: string;
  onChange: (value: string) => void;
  busy: boolean;
  authorized: boolean;
  onAuthorize: () => Promise<boolean>;
  onPrepare: () => void;
  onSend: (value: string) => void;
  onHandoff: () => void;
}) {
  const { t } = useLocale();
  const epoch = useDevelopment(state => state.epoch);
  const [configuration, setConfiguration] = useState<SessionConfigurationView | null>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const [queued, setQueued] = useState(false);
  useEffect(() => { if (!busy) setQueued(false); }, [busy]);
  const send = () => { if (!queued && value.trim()) { setQueued(true); onSend(value); } };
  const resolved = resolveAgentDefaults(session.project_id, session.id);
  const defaults = resolved.source === 'session' ? resolved : null;
  useEffect(() => {
    let active = true;
    setConfiguration(null);
    void readSessionConfiguration(session.id).then(value => { if (active && epoch === useDevelopment.getState().epoch) setConfiguration(value); }, () => { /* Retained history remains readable without a configuration receipt. */ });
    return () => { active = false; };
  }, [session.id, epoch]);
  const binding = configuration?.format === 'provider_bound' ? configuration.snapshot : null;
  // A closed Provider binding is read-only until the host revalidates it on resume;
  // only legacy history lacks that resume contract entirely.
  const legacy = configuration?.format === 'legacy_history';
  return <div className="agent-dialogue-composer" data-testid="saved-conversation-composer"><div className="agent-composer-shell">
    <p className="m-0 text-xs text-ink-secondary">{t('desktop.historyHint')}</p>
    <div className="agent-composer-input"><Input ref={field} multiline rows={2} label={t('agent.prompt')} value={value} disabled={legacy} onChange={event => onChange(event.target.value)} onFocus={() => { if (authorized && !busy && !legacy) onPrepare(); }} onKeyDown={event => {
      if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing && !event.repeat && !queued && value.trim()) { event.preventDefault(); send(); }
    }} /></div>
    <div className="agent-composer-controls"><div className="agent-control-leading"><span>{session.engine === 'codex' ? 'Codex' : 'Claude'}</span><span>{binding ? `${binding.provider_id} / ${binding.model_id}` : defaults?.configuration ? `${defaults.configuration.provider.provider_id} / ${defaults.configuration.provider.model_id}` : session.model ?? t('agentControls.unknown')}</span>{defaults?.settings.reasoning_effort ? <span>{defaults.settings.reasoning_effort}</span> : null}</div>
      <div className="agent-control-trailing"><Button size="xs" variant="ghost" disabled={busy} onClick={onHandoff}>{t('workbenchNavigation.continueModel')}</Button><Button disabled={queued || legacy || !value.trim()} onClick={send}>{t(queued ? 'desktop.queuedFirst' : 'desktop.continue')}</Button></div>
    </div>
    {!authorized && !legacy ? <Button size="xs" variant="secondary" disabled={busy} onClick={() => { void onAuthorize().then(ok => { if (ok) onPrepare(); }); }}>{t('desktop.trust.start')}</Button> : null}
    <details className="dev-context-details"><summary>{t('desktop.recovery')}</summary><nav className="dev-review-toolbar"><WorkspaceLink to="/settings/providers">{t('capabilities.provider')}</WorkspaceLink><WorkspaceLink to="/settings/cli">{t('agent.engine')}</WorkspaceLink><Button size="xs" variant="ghost" disabled={busy || legacy} onClick={onPrepare}>{t('desktop.retryPrepare')}</Button></nav></details>
  </div></div>;
}

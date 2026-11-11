import { useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { DevelopmentSession } from '../../stores/developmentStore';
import { useAgentRunner } from '../../stores/agentRunnerStore';

// Browsing a saved session never classifies it, starts a CLI, or grants workspace trust.
export function UnclassifiedPanel({ sessions, busy, onOpen }: { sessions: DevelopmentSession[]; busy: boolean; onOpen(session: DevelopmentSession): void }) {
  const { t } = useLocale();
  const [search, setSearch] = useState('');
  const live = useAgentRunner(state => state.live);
  const selectedId = useAgentRunner(state => state.selected);
  const query = search.trim().toLocaleLowerCase();
  const matchingIds = new Set(sessions.filter(session => [session.engine, session.workspace, session.id, session.native_id ?? '', session.model ?? ''].some(value => value.toLocaleLowerCase().includes(query))).map(session => session.id));
  const visible = sessions.filter(session => session.id === selectedId || matchingIds.has(session.id));
  if (!sessions.length) return null;
  return <section aria-label={t('dev.unclassified')} className="dev-panel dev-nav-panel" data-testid="dev-unclassified">
    <details className="dev-disclosure" open>
      <summary className="dev-disclosure-summary">{t('dev.unclassified')} <span className="dev-nav-status">{sessions.length}</span></summary>
      <div className="dev-disclosure-body">
    <p className="m-0 text-xs text-ink-secondary">{t('workbenchNavigation.recoveryPanelHint')}</p>
    <div className="dev-nav-search"><Input type="search" data-testid="dev-unclassified-search" label={t('workbenchNavigation.searchUnclassified')} placeholder={t('workbenchNavigation.sessionSearchHint')} value={search} onChange={event => setSearch(event.target.value)} />{search ? <Button size="xs" variant="ghost" onClick={() => setSearch('')}>{t('patterns.clearSearch')}</Button> : null}</div>
    {query && !matchingIds.size ? <p role="status" className="m-0 text-xs text-ink-secondary">{t('workbenchNavigation.noMatches')}</p> : null}
    <div className="dev-session-list grid gap-1">
    {visible.map(session => <div key={session.id} className="dev-session-item flex items-center gap-2 text-xs" data-testid={`dev-session-unclassified-${session.id}`}>
      <span className="dev-nav-item-label min-w-0 flex-1 truncate" title={`${session.id}\n${session.workspace}`}>{session.engine === 'codex' ? 'Codex' : 'Claude'} · {session.id.slice(0, 8)} · {session.workspace}</span>
      {selectedId === session.id ? <span className="dev-nav-status">{t('workbenchNavigation.current')}</span> : null}
      {live[session.id] ? <span className="dev-nav-status">{t(`agent.state.${live[session.id]}`)}</span> : null}
      <Button size="xs" variant="secondary" data-testid={`dev-session-recover-${session.id}`} disabled={busy} onClick={() => onOpen(session)}>{t('workbenchNavigation.selectSession')}</Button>
    </div>)}
    </div>
      </div>
    </details>
  </section>;
}

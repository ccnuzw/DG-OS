import { useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import { useDevelopment, type DevelopmentSession } from '../../stores/developmentStore';

// Project history is independent of the open work surfaces. Browsing never resumes a CLI.
export function SessionPanel({ busy, onSelect }: { busy: boolean; onSelect: (session: DevelopmentSession) => void }) {
  const { t } = useLocale();
  const { projects, tasks, sessions, selection, sessionNames } = useDevelopment();
  const { live, unread, snapshots } = useAgentRunner();
  const [search, setSearch] = useState('');
  const query = search.trim().toLocaleLowerCase();
  const title = (session: DevelopmentSession) => sessionNames[session.id] || tasks.find(task => task.id === session.task_id)?.goal || t('workbenchNavigation.sessionLabel');
  const matching = sessions.filter(session => [title(session), session.engine, session.model ?? '', session.id, projects.find(project => project.id === session.project_id)?.name ?? ''].some(value => value.toLocaleLowerCase().includes(query)));
  return <section className="dev-panel dev-session-panel" aria-label={t('dev.sessions')} data-testid="dev-session-list">
    <header className="dev-panel-header"><h2>{t('dev.sessions')}</h2></header>
    <Input type="search" data-testid="dev-session-search" label={t('workbenchNavigation.searchSessions')} value={search} onChange={event => setSearch(event.target.value)} />
    {!matching.length ? <p>{t(query ? 'workbenchNavigation.noMatches' : 'dev.noSessions')}</p> : null}
    {projects.map(project => {
      const grouped = matching.filter(session => session.project_id === project.id);
      if (!grouped.length) return null;
      return <section key={project.id} className="dev-session-project" aria-label={project.name}><h3>{project.name}{project.archived ? ` · ${t('dev.projectStatus.archived')}` : ''}</h3>
        {grouped.map(session => <div key={session.id} className="dev-session-item" data-testid={`dev-session-item-${session.id}`}>
          <Button size="xs" variant={selection.session_id === session.id ? 'secondary' : 'ghost'} className="dev-session-select" aria-pressed={selection.session_id === session.id} disabled={busy} onClick={() => onSelect(session)}>
            <span className="dev-nav-item-label">{title(session)}</span>
            <span className="dev-session-meta">{session.model ?? session.engine}{live[session.id] ? ` · ${t(`agent.state.${live[session.id]}`)}` : ''}{snapshots[session.id]?.pending.length ? ` · ${t('workbenchNavigation.waiting')}` : unread[session.id] ? ` · ${t('workbenchNavigation.newReply')}` : ''}</span>
          </Button>
        </div>)}
      </section>;
    })}
  </section>;
}

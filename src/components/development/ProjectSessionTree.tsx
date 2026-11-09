import { useEffect, useId, useMemo, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { ChevronDown, ChevronRight, Ellipsis, Folder, FolderOpen, MessageSquare, Plus } from 'lucide-react';
import { formatDateTime, useLocale } from '../../i18n';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import { useDevelopment, type ConversationTab, type DevelopmentSession, type Project } from '../../stores/developmentStore';
import { readDevelopmentSessionSummaries, type SessionSummary } from '../../stores/developmentExplorer';
import { ProjectPanel } from './ProjectPanel';
import './project-navigation.css';

export interface ProjectSessionTreeProps {
  busy: boolean;
  onSelect: (session: DevelopmentSession) => void;
  onNewConversation: (projectId: string) => void;
  onSelectProject: (projectId: string) => void;
  onSelectDraft: (tab: ConversationTab) => void;
  onManageProject: (projectId: string) => void;
}

const PAGE_SIZE = 5;
const UNCLASSIFIED = '__unclassified__';

/** Navigation is read-only: selection is delegated, expansion never changes context. */
export function ProjectSessionTree({ busy, onSelect, onNewConversation, onSelectProject, onSelectDraft, onManageProject }: ProjectSessionTreeProps) {
  const { t, locale } = useLocale();
  const { projects, tasks, sessions, selection, sessionNames, tabs, activeTabId, epoch, workspaceOwner } = useDevelopment();
  const snapshots = useAgentRunner(state => state.snapshots);
  const live = useAgentRunner(state => state.live);
  const unread = useAgentRunner(state => state.unread);
  const [query, setQuery] = useState('');
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [shown, setShown] = useState<Record<string, number>>({});
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [unclassifiedOpen, setUnclassifiedOpen] = useState(false);
  const [metadata, setMetadata] = useState<{ epoch: number; owner: string | null; rows: SessionSummary[]; failed: boolean } | null>(null);
  const treeId = useId();
  const normalized = query.trim().toLocaleLowerCase();

  useEffect(() => {
    let current = true;
    // Coalesce runtime message updates into one caller-scoped read, never one read per row.
    const timer = window.setTimeout(() => {
      void readDevelopmentSessionSummaries().then(result => {
        const state = useDevelopment.getState();
        if (!current || state.epoch !== epoch || state.workspaceOwner !== workspaceOwner) return;
        setMetadata({ epoch, owner: workspaceOwner, rows: result.ok ? result.value : [], failed: !result.ok });
      });
    }, 200);
    return () => { current = false; window.clearTimeout(timer); };
  }, [epoch, workspaceOwner, sessions, snapshots]);

  const summaries = useMemo(() => new Map<string, SessionSummary>(metadata?.epoch === epoch && metadata.owner === workspaceOwner ? metadata.rows.map(row => [row.session_id, row]) : []), [metadata, epoch, workspaceOwner]);
  const taskTitles = useMemo(() => new Map(tasks.map(task => [task.id, task.goal])), [tasks]);
  const projectIds = useMemo(() => new Set(projects.map(project => project.id)), [projects]);
  const grouped = useMemo(() => {
    const result = new Map<string, DevelopmentSession[]>();
    for (const session of sessions) {
      const key = session.project_id && projectIds.has(session.project_id) ? session.project_id : UNCLASSIFIED;
      const rows = result.get(key) ?? [];
      rows.push(session);
      result.set(key, rows);
    }
    for (const rows of result.values()) rows.sort((a, b) => (summaries.get(b.id)?.last_activity_at ?? 0) - (summaries.get(a.id)?.last_activity_at ?? 0));
    return result;
  }, [sessions, projectIds, summaries]);
  const title = (session: DevelopmentSession) => sessionNames[session.id] || (session.task_id ? taskTitles.get(session.task_id) : null) || t('workbenchNavigation.sessionLabel');
  const includes = (...values: (string | null | undefined)[]) => values.some(value => value?.toLocaleLowerCase().includes(normalized));
  const sessionMatches = (session: DevelopmentSession) => includes(title(session), summaries.get(session.id)?.preview, session.engine, session.model, session.id, session.workspace);
  const matchingProjectIds = new Set(projects.filter(project => includes(project.name, project.path, project.id)).map(project => project.id));
  const draftTabs = useMemo(() => {
    const result = new Map<string, ConversationTab[]>();
    for (const tab of tabs) {
      if (tab.sessionId) continue;
      const rows = result.get(tab.projectId) ?? [];
      rows.push(tab);
      result.set(tab.projectId, rows);
    }
    return result;
  }, [tabs]);
  const visibleProjects = projects.filter(project => !normalized || project.id === selection.project_id || matchingProjectIds.has(project.id) || (grouped.get(project.id) ?? []).some(sessionMatches) || (draftTabs.get(project.id) ?? []).some(tab => includes(tab.title, t('workbenchNavigation.unsentDraft'))));
  const activeProjects = visibleProjects.filter(project => !project.archived).sort((a, b) => (b.last_opened_at ?? b.created_at) - (a.last_opened_at ?? a.created_at));
  const archivedProjects = visibleProjects.filter(project => project.archived);
  const unclassified = grouped.get(UNCLASSIFIED) ?? [];
  const visibleUnclassified = unclassified.filter(session => !normalized || session.id === selection.session_id || sessionMatches(session));
  const selectedUnclassified = unclassified.some(session => session.id === selection.session_id);

  function renderSessions(key: string, rows: DevelopmentSession[], drafts: ConversationTab[], allMatch = false) {
    const matching = rows.filter(session => !normalized || allMatch || session.id === selection.session_id || sessionMatches(session));
    const limit = shown[key] ?? PAGE_SIZE;
    const visible = normalized ? matching : matching.filter((session, index) => index < limit || session.id === selection.session_id);
    const visibleDrafts = drafts.filter(tab => !normalized || allMatch || tab.id === activeTabId || includes(tab.title, t('workbenchNavigation.unsentDraft')));
    const remaining = matching.length - visible.length;
    return <div className="dev-tree-session-list">
      {visibleDrafts.map(tab => <Button key={tab.id} size="xs" variant="ghost" className={`dev-tree-session is-draft${activeTabId === tab.id ? ' is-current' : ''}`} aria-pressed={activeTabId === tab.id} disabled={busy} data-testid={`dev-draft-item-${tab.id}`} onClick={() => onSelectDraft(tab)}>
        <MessageSquare size={14} aria-hidden="true" /><span className="dev-tree-session-main"><strong>{tab.title || t('workbenchNavigation.blank')}</strong><small>{t('workbenchNavigation.unsentDraft')}</small></span>
      </Button>)}
      {visible.map(session => {
        const summary = summaries.get(session.id);
        const timestamp = summary?.last_activity_at;
        const date = timestamp != null && Number.isFinite(timestamp) ? new Date(timestamp * 1000) : null;
        const validDate = date && Number.isFinite(date.getTime()) ? date : null;
        const snapshot = snapshots[session.id];
        const state = snapshot?.pending.length ? t('workbenchNavigation.waiting') : (live[session.id] ?? snapshot?.status) === 'running' ? t('workbenchNavigation.runningTab') : unread[session.id] ? t('workbenchNavigation.newReply') : '';
        const current = selection.session_id === session.id;
        return <Button key={session.id} size="xs" variant="ghost" className={`dev-tree-session${current ? ' is-current' : ''}`} aria-pressed={current} disabled={busy} data-testid={`dev-session-item-${session.id}`} onClick={() => onSelect(session)}>
          <MessageSquare size={14} aria-hidden="true" />
          <span className="dev-tree-session-main"><strong title={title(session)}>{title(session)}</strong><small title={summary?.preview ?? undefined}>{summary?.preview || t('workbenchNavigation.noPreview')}</small>{state ? <span className="dev-tree-session-status">{state}</span> : null}</span>
          {validDate ? <time dateTime={validDate.toISOString()} title={formatDateTime(validDate, locale)}>{new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' }).format(validDate)}</time> : null}
        </Button>;
      })}
      {!matching.length && !visibleDrafts.length ? <p className="dev-tree-empty">{t(normalized ? 'workbenchNavigation.noMatchingSessions' : 'workbenchNavigation.noProjectSessions')}</p> : null}
      {remaining > 0 ? <Button size="xs" variant="ghost" className="dev-tree-more" onClick={() => setShown(value => ({ ...value, [key]: limit + PAGE_SIZE }))}>{t('workbenchNavigation.showMoreSessions', { count: remaining })}</Button> : null}
    </div>;
  }

  function renderProject(project: Project) {
    const open = Boolean(normalized) || (expanded[project.id] ?? project.id === selection.project_id);
    const rowsId = `${treeId}-${project.id}`;
    return <section key={project.id} className="dev-tree-project" data-project-id={project.id}>
      <header className={`dev-tree-project-row${selection.project_id === project.id ? ' is-current' : ''}`}>
        <Button size="xs" variant="ghost" className="dev-tree-toggle" aria-label={`${t(open ? 'workbenchNavigation.collapseProject' : 'workbenchNavigation.expandProject')}: ${project.name}`} aria-expanded={open} aria-controls={rowsId} onClick={() => setExpanded(value => ({ ...value, [project.id]: !open }))}>{open ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />}</Button>
        <Button size="xs" variant="ghost" className="dev-tree-project-name" data-testid={`dev-project-item-${project.id}`} disabled={busy} aria-pressed={selection.project_id === project.id} onClick={() => onSelectProject(project.id)}>{open ? <FolderOpen size={16} aria-hidden="true" /> : <Folder size={16} aria-hidden="true" />}<span title={project.path}>{project.name}</span></Button>
        <Button size="xs" variant="ghost" className="dev-tree-icon-action" aria-label={`${t('dev.newSession')}: ${project.name}`} title={t('dev.newSession')} disabled={busy || project.archived} onClick={() => onNewConversation(project.id)}><Plus size={15} aria-hidden="true" /></Button>
        <Button size="xs" variant="ghost" className="dev-tree-icon-action" aria-label={`${t('workbenchNavigation.manageProject')}: ${project.name}`} title={t('workbenchNavigation.manageProject')} disabled={busy} onClick={() => onManageProject(project.id)}><Ellipsis size={15} aria-hidden="true" /></Button>
      </header>
      <div id={rowsId} hidden={!open}>{open ? renderSessions(project.id, grouped.get(project.id) ?? [], draftTabs.get(project.id) ?? [], Boolean(normalized) && matchingProjectIds.has(project.id)) : null}</div>
    </section>;
  }

  return <section className="dev-project-session-tree" data-testid="dev-project-session-tree" aria-label={t('workbenchNavigation.projectsAndSessions')}>
    <header className="dev-tree-header"><h2>{t('dev.projects')}</h2><ProjectPanel mode="add-only" /></header>
    <div className="dev-tree-search"><Input type="search" data-testid="dev-unified-search" aria-label={t('workbenchNavigation.searchProjectsAndSessions')} placeholder={t('workbenchNavigation.searchProjectsAndSessions')} value={query} onChange={event => setQuery(event.target.value)} /></div>
    {metadata?.epoch === epoch && metadata.owner === workspaceOwner && metadata.failed ? <p className="dev-tree-empty" role="status">{t('workbenchNavigation.metadataUnavailable')}</p> : null}
    {!visibleProjects.length && !visibleUnclassified.length ? <p className="dev-tree-empty">{t(normalized ? 'workbenchNavigation.noMatches' : 'dev.none')}</p> : null}
    <nav className="dev-tree-list" aria-label={t('dev.projects')}>
      {activeProjects.map(renderProject)}
      {archivedProjects.length ? <section className="dev-tree-history"><Button size="xs" variant="ghost" className="dev-tree-disclosure" aria-expanded={archivedOpen || Boolean(normalized) || archivedProjects.some(project => project.id === selection.project_id)} aria-controls={`${treeId}-archived`} onClick={() => setArchivedOpen(value => !value)}><ChevronRight size={14} aria-hidden="true" />{t('dev.archived')}<span>{archivedProjects.length}</span></Button><div id={`${treeId}-archived`} hidden={!(archivedOpen || Boolean(normalized) || archivedProjects.some(project => project.id === selection.project_id))}>{archivedProjects.map(renderProject)}</div></section> : null}
      {visibleUnclassified.length ? <section className="dev-tree-history"><Button size="xs" variant="ghost" className="dev-tree-disclosure" aria-expanded={unclassifiedOpen || Boolean(normalized) || selectedUnclassified} aria-controls={`${treeId}-unclassified`} onClick={() => setUnclassifiedOpen(value => !value)}><ChevronRight size={14} aria-hidden="true" />{t('dev.unclassified')}<span>{visibleUnclassified.length}</span></Button><div id={`${treeId}-unclassified`} hidden={!(unclassifiedOpen || Boolean(normalized) || selectedUnclassified)}>{renderSessions(UNCLASSIFIED, visibleUnclassified, [])}</div></section> : null}
    </nav>
  </section>;
}

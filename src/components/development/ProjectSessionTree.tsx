import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { ChevronRight, Folder, FolderOpen, MessageSquare, Pin, Plus } from 'lucide-react';
import { formatDateTime, useLocale } from '../../i18n';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import { useDevelopment, type ConversationTab, type DevelopmentSession, type Project } from '../../stores/developmentStore';
import { readDevelopmentSessionSummaries, searchDevelopmentMessages, type ConversationSearchMatch, type ConversationSearchResult, type SessionSummary } from '../../stores/developmentExplorer';
import { ProjectActions } from './ProjectActions';
import './project-navigation.css';

export interface ConversationRevealTarget {
  tabId: string;
  projectId: string;
  sessionId: string | null;
  nonce: number;
}

export interface ProjectSessionTreeProps {
  busy: boolean;
  onSelect: (session: DevelopmentSession, match?: ConversationSearchMatch) => void;
  onNewConversation: (projectId: string) => void;
  onSelectProject: (projectId: string) => void;
  onSelectDraft: (tab: ConversationTab) => void;
  onRefreshProject: (projectId: string) => Promise<void>;
  onProjectFiles: (projectId: string) => Promise<void>;
  onGoals?: (projectId: string) => void;
  revealTarget?: ConversationRevealTarget | null;
}

const PAGE_SIZE = 5;
const UNCLASSIFIED = '__unclassified__';

/** Selection is delegated; folding and archived-history disclosure never change context. */
export function ProjectSessionTree({ busy, onSelect, onNewConversation, onSelectProject, onSelectDraft, onRefreshProject, onProjectFiles, onGoals, revealTarget }: ProjectSessionTreeProps) {
  const { t, locale } = useLocale();
  const { projects, tasks, sessions, selection, sessionNames, tabs, activeTabId, epoch, workspaceOwner, pinnedProjectIds, archivedSessionIds } = useDevelopment();
  const snapshots = useAgentRunner(state => state.snapshots);
  const live = useAgentRunner(state => state.live);
  const unread = useAgentRunner(state => state.unread);
  const [query, setQuery] = useState('');
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [shown, setShown] = useState<Record<string, number>>({});
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [archivedConversationsOpen, setArchivedConversationsOpen] = useState<Record<string, boolean>>({});
  const [unclassifiedOpen, setUnclassifiedOpen] = useState(false);
  const [metadata, setMetadata] = useState<{ epoch: number; owner: string | null; rows: SessionSummary[]; failed: boolean } | null>(null);
  const [search, setSearch] = useState<{ query: string; epoch: number; owner: string | null; loading: boolean; result: ConversationSearchResult | null } | null>(null);
  const [closedDraftsOpen, setClosedDraftsOpen] = useState(false);
  const scheduleSummaryRead = useRef<(() => void) | null>(null);
  const root = useRef<HTMLElement>(null);
  const [revealed, setRevealed] = useState<{ target: ConversationRevealTarget; epoch: number; owner: string | null } | null>(null);
  const pendingReveal = useRef<ConversationRevealTarget | null>(null);
  const treeId = useId();
  const normalized = query.trim().toLocaleLowerCase();

  useEffect(() => {
    let current = true;
    let timer: number | null = null;
    let sequence = 0;
    let applied = 0;
    // Keep the first scheduled batch instead of restarting it on every runtime snapshot.
    const schedule = () => {
      if (timer !== null || document.hidden) return;
      timer = window.setTimeout(() => {
        timer = null;
        const read = ++sequence;
        void readDevelopmentSessionSummaries().then(result => {
          const state = useDevelopment.getState();
          if (!current || read <= applied || document.hidden || state.epoch !== epoch || state.workspaceOwner !== workspaceOwner) return;
          applied = read;
          setMetadata({ epoch, owner: workspaceOwner, rows: result.ok ? result.value : [], failed: !result.ok });
        });
      }, 200);
    };
    const visibility = () => {
      applied = ++sequence;
      if (timer !== null) window.clearTimeout(timer);
      timer = null;
      if (!document.hidden) schedule();
    };
    scheduleSummaryRead.current = schedule;
    document.addEventListener('visibilitychange', visibility);
    return () => {
      current = false;
      if (timer !== null) window.clearTimeout(timer);
      scheduleSummaryRead.current = null;
      document.removeEventListener('visibilitychange', visibility);
    };
  }, [epoch, workspaceOwner]);

  useEffect(() => {
    scheduleSummaryRead.current?.();
  }, [epoch, workspaceOwner, sessions, snapshots]);


  useEffect(() => {
    if (!normalized) { setSearch(null); return; }
    const controller = new AbortController();
    const state = { query: normalized, epoch, owner: workspaceOwner };
    const current = () => !controller.signal.aborted && useDevelopment.getState().epoch === epoch && useDevelopment.getState().workspaceOwner === workspaceOwner;
    setSearch({ ...state, loading: true, result: null });
    const timer = window.setTimeout(() => {
      void searchDevelopmentMessages(sessions, normalized, controller.signal).then(result => {
        if (current()) setSearch({ ...state, loading: false, result });
      });
    }, 250);
    return () => { window.clearTimeout(timer); controller.abort(); };
  }, [normalized, epoch, workspaceOwner, sessions]);

  const messageSearch = search?.query === normalized && search.epoch === epoch && search.owner === workspaceOwner ? search : null;
  const messageMatches = useMemo(() => new Map(messageSearch?.result?.matches.map(match => [match.sessionId, match]) ?? []), [messageSearch]);
  const closedDrafts = tabs.filter(tab => tab.hidden && !tab.sessionId && projects.some(project => project.id === tab.projectId));
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
  const sessionMatches = (session: DevelopmentSession) => messageMatches.has(session.id) || includes(title(session), summaries.get(session.id)?.preview, session.engine, session.model, session.id, session.workspace);
  const matchingProjectIds = new Set(projects.filter(project => includes(project.name, project.path, project.id)).map(project => project.id));
  const draftTabs = useMemo(() => {
    const result = new Map<string, ConversationTab[]>();
    for (const tab of tabs) {
      if (tab.sessionId || tab.hidden) continue;
      const rows = result.get(tab.projectId) ?? [];
      rows.push(tab);
      result.set(tab.projectId, rows);
    }
    return result;
  }, [tabs]);
  const visibleProjects = projects.filter(project => !normalized || project.id === selection.project_id || matchingProjectIds.has(project.id) || (grouped.get(project.id) ?? []).some(sessionMatches) || (draftTabs.get(project.id) ?? []).some(tab => includes(tab.title, t('workbenchNavigation.unsentDraft'))));
  const activeProjects = visibleProjects.filter(project => !project.archived).sort((a, b) => Number(pinnedProjectIds.includes(b.id)) - Number(pinnedProjectIds.includes(a.id)) || (b.last_opened_at ?? b.created_at) - (a.last_opened_at ?? a.created_at));
  const archivedProjects = visibleProjects.filter(project => project.archived);
  const unclassified = grouped.get(UNCLASSIFIED) ?? [];
  const visibleUnclassified = unclassified.filter(session => !normalized || session.id === selection.session_id || sessionMatches(session));
  const selectedUnclassified = unclassified.some(session => session.id === selection.session_id);

  useEffect(() => {
    if (!revealTarget) return;
    const session = revealTarget.sessionId ? sessions.find(session => session.id === revealTarget.sessionId) : null;
    const projectId = session ? session.project_id : revealTarget.projectId;
    const group = projectId && projectIds.has(projectId) ? projectId : UNCLASSIFIED;
    setQuery('');
    setRevealed({ target: revealTarget, epoch, owner: workspaceOwner });
    pendingReveal.current = revealTarget;
    if (group === UNCLASSIFIED) setUnclassifiedOpen(true);
    else {
      setExpanded(value => ({ ...value, [group]: true }));
      if (projects.find(project => project.id === group)?.archived) setArchivedOpen(true);
      if (revealTarget.sessionId && archivedSessionIds.includes(revealTarget.sessionId)) setArchivedConversationsOpen(value => ({ ...value, [group]: true }));
    }
  }, [revealTarget]);

  useEffect(() => {
    const target = pendingReveal.current;
    if (!target || !revealed || revealed.epoch !== epoch || revealed.owner !== workspaceOwner || busy || normalized) return;
    const item = Array.from(root.current?.querySelectorAll<HTMLElement>('[data-conversation-tab-id], [data-conversation-session-id]') ?? []).find(item => target.sessionId ? item.dataset.conversationSessionId === target.sessionId : item.dataset.conversationTabId === target.tabId);
    if (!item || item.closest('[hidden]')) return;
    item.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
    item.focus({ preventScroll: true });
    pendingReveal.current = null;
  }, [revealed, epoch, workspaceOwner, busy, normalized, expanded, archivedOpen, archivedConversationsOpen, unclassifiedOpen, sessions]);

  const revealedSessionId = revealed?.epoch === epoch && revealed.owner === workspaceOwner ? revealed.target.sessionId : null;

  function renderSessions(key: string, rows: DevelopmentSession[], drafts: ConversationTab[], allMatch = false) {
    const matching = rows.filter(session => !normalized || allMatch || session.id === selection.session_id || sessionMatches(session));
    const limit = shown[key] ?? PAGE_SIZE;
    const visible = normalized ? matching : matching.filter((session, index) => index < limit || session.id === selection.session_id || session.id === revealedSessionId);
    const visibleDrafts = drafts.filter(tab => !normalized || allMatch || tab.id === activeTabId || includes(tab.title, t('workbenchNavigation.unsentDraft')));
    const remaining = matching.length - visible.length;
    return <div className="dev-tree-session-list">
      {visibleDrafts.map(tab => <Button key={tab.id} size="xs" variant="ghost" className={`dev-tree-session is-draft${activeTabId === tab.id ? ' is-current' : ''}`} aria-pressed={activeTabId === tab.id} disabled={busy} data-testid={`dev-draft-item-${tab.id}`} data-conversation-tab-id={tab.id} onClick={() => onSelectDraft(tab)}>
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
        const match = normalized ? messageMatches.get(session.id) : undefined;
        const preview = match ? match.text.slice(Math.max(0, match.text.toLocaleLowerCase().indexOf(normalized) - 50), Math.max(0, match.text.toLocaleLowerCase().indexOf(normalized) - 50) + 240) : summary?.preview;
        return <Button key={session.id} size="xs" variant="ghost" className={`dev-tree-session${current ? ' is-current' : ''}`} aria-pressed={current} disabled={busy} data-testid={`dev-session-item-${session.id}`} data-conversation-session-id={session.id} onClick={() => match ? onSelect(session, match) : onSelect(session)}>
          <MessageSquare size={14} aria-hidden="true" />
          <span className="dev-tree-session-main"><strong title={title(session)}>{title(session)}</strong><small className={match ? 'dev-tree-message-match' : undefined} title={preview ?? undefined}>{preview || t('workbenchNavigation.noPreview')}</small>{match ? <span className="dev-tree-match-label">{t('workbenchNavigation.messageMatch')}</span> : null}{state ? <span className="dev-tree-session-status">{state}</span> : null}</span>
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
    const rows = grouped.get(project.id) ?? [];
    const archived = rows.filter(session => archivedSessionIds.includes(session.id) && session.id !== selection.session_id && !tabs.some(tab => tab.sessionId === session.id) && (!live[session.id] || live[session.id] === 'closed'));
    const archivedIds = new Set(archived.map(session => session.id));
    const historyOpen = Boolean(normalized) || Boolean(archivedConversationsOpen[project.id]);
    return <section key={project.id} className="dev-tree-project" data-project-id={project.id}>
      <header className={`dev-tree-project-row${selection.project_id === project.id ? ' is-current' : ''}`}>
        <Button size="xs" variant="ghost" className="dev-tree-toggle" aria-label={`${t(open ? 'workbenchNavigation.collapseProject' : 'workbenchNavigation.expandProject')}: ${project.name}`} aria-expanded={open} aria-controls={rowsId} onClick={() => setExpanded(value => ({ ...value, [project.id]: !open }))}>{open ? <FolderOpen size={16} aria-hidden="true" /> : <Folder size={16} aria-hidden="true" />}</Button>
        <Button size="xs" variant="ghost" className="dev-tree-project-name" data-testid={`dev-project-item-${project.id}`} disabled={busy} aria-pressed={selection.project_id === project.id} onClick={() => onSelectProject(project.id)}><span title={project.path}>{project.name}</span>{pinnedProjectIds.includes(project.id) ? <Pin size={12} aria-label={t('workbenchNavigation.pinned')} /> : null}</Button>
        <Button size="xs" variant="ghost" className="dev-tree-icon-action" data-testid={`dev-project-new-${project.id}`} aria-label={`${t('dev.newSession')}: ${project.name}`} title={t('dev.newSession')} disabled={busy || project.archived} onClick={() => onNewConversation(project.id)}><Plus size={15} aria-hidden="true" /></Button>
        <ProjectActions project={project} disabled={busy} onRefresh={onRefreshProject} onFiles={onProjectFiles} onGoals={onGoals} />
      </header>
      <div id={rowsId} hidden={!open}>{open ? <>
        {renderSessions(project.id, rows.filter(session => !archivedIds.has(session.id)), draftTabs.get(project.id) ?? [], Boolean(normalized) && matchingProjectIds.has(project.id))}
        {archived.length ? <section className="dev-tree-history"><Button size="xs" variant="ghost" className="dev-tree-disclosure" aria-expanded={historyOpen} aria-controls={`${rowsId}-history`} onClick={() => setArchivedConversationsOpen(value => ({ ...value, [project.id]: !historyOpen }))}><ChevronRight size={14} aria-hidden="true" />{t('workbenchNavigation.archivedConversations')}<span>{archived.length}</span></Button><div id={`${rowsId}-history`} hidden={!historyOpen}>{renderSessions(`${project.id}-archive`, archived, [], Boolean(normalized) && matchingProjectIds.has(project.id))}<Button size="xs" variant="ghost" disabled={busy} onClick={() => useDevelopment.getState().restoreSessions(archived.map(session => session.id))}>{t('workbenchNavigation.restoreConversations')}</Button></div></section> : null}
      </> : null}</div>
    </section>;
  }

  return <section ref={root} className="dev-project-session-tree" data-testid="dev-project-session-tree" aria-label={t('workbenchNavigation.projectsAndSessions')}>
    <div className="dev-tree-search"><Input type="search" data-testid="dev-unified-search" aria-label={t('workbenchNavigation.searchProjectsAndSessions')} placeholder={t('workbenchNavigation.searchProjectsAndSessions')} value={query} onChange={event => setQuery(event.target.value)} /></div>
    {normalized ? <div className="dev-tree-search-feedback" role="status" aria-live="polite">
      <p>{t(messageSearch?.loading || !messageSearch ? 'workbenchNavigation.searchingMessages' : 'workbenchNavigation.messageSearchScope')}</p>
      {messageSearch?.result?.truncated ? <p>{t('workbenchNavigation.messageSearchTruncated')}</p> : null}
      {messageSearch?.result?.unavailable ? <p>{t('workbenchNavigation.messageSearchUnavailable', { count: messageSearch.result.unavailable })}</p> : null}
    </div> : null}
    {metadata?.epoch === epoch && metadata.owner === workspaceOwner && metadata.failed ? <p className="dev-tree-empty" role="status">{t('workbenchNavigation.metadataUnavailable')}</p> : null}
    {!visibleProjects.length && !visibleUnclassified.length ? <p className="dev-tree-empty">{t(normalized ? 'workbenchNavigation.noMatches' : 'dev.none')}</p> : null}
    <nav className="dev-tree-list" aria-label={t('dev.projects')}>
      {activeProjects.map(renderProject)}
      {closedDrafts.length ? <section className="dev-tree-history"><Button size="xs" variant="ghost" className="dev-tree-disclosure" aria-expanded={closedDraftsOpen || Boolean(normalized)} aria-controls={`${treeId}-closed-drafts`} onClick={() => setClosedDraftsOpen(value => !value)}><ChevronRight size={14} aria-hidden="true" />{t('workbenchNavigation.closedDrafts')}<span>{closedDrafts.length}</span></Button><div id={`${treeId}-closed-drafts`} hidden={!(closedDraftsOpen || Boolean(normalized))} className="dev-tree-session-list">{closedDrafts.filter(tab => !normalized || includes(tab.title, projects.find(project => project.id === tab.projectId)?.name, t('workbenchNavigation.unsentDraft'))).map(tab => <Button key={tab.id} size="xs" variant="ghost" className="dev-tree-session is-draft" disabled={busy} data-testid={`dev-closed-draft-${tab.id}`} onClick={() => onSelectDraft(tab)}><MessageSquare size={14} aria-hidden="true" /><span className="dev-tree-session-main"><strong>{tab.title || t('workbenchNavigation.blank')}</strong><small>{projects.find(project => project.id === tab.projectId)?.name} · {t('workbenchNavigation.restoreDraft')}</small></span></Button>)}</div></section> : null}
      {archivedProjects.length ? <section className="dev-tree-history"><Button size="xs" variant="ghost" className="dev-tree-disclosure" aria-expanded={archivedOpen || Boolean(normalized) || archivedProjects.some(project => project.id === selection.project_id)} aria-controls={`${treeId}-archived`} onClick={() => setArchivedOpen(value => !value)}><ChevronRight size={14} aria-hidden="true" />{t('dev.archived')}<span>{archivedProjects.length}</span></Button><div id={`${treeId}-archived`} hidden={!(archivedOpen || Boolean(normalized) || archivedProjects.some(project => project.id === selection.project_id))}>{archivedProjects.map(renderProject)}</div></section> : null}
      {visibleUnclassified.length ? <section className="dev-tree-history"><Button size="xs" variant="ghost" className="dev-tree-disclosure" aria-expanded={unclassifiedOpen || Boolean(normalized) || selectedUnclassified} aria-controls={`${treeId}-unclassified`} onClick={() => setUnclassifiedOpen(value => !value)}><ChevronRight size={14} aria-hidden="true" />{t('dev.unclassified')}<span>{visibleUnclassified.length}</span></Button><div id={`${treeId}-unclassified`} hidden={!(unclassifiedOpen || Boolean(normalized) || selectedUnclassified)}>{renderSessions(UNCLASSIFIED, visibleUnclassified, [])}</div></section> : null}
    </nav>
  </section>;
}

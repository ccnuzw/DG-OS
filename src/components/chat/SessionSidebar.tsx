import { useEffect, useRef, useState } from 'react';
import { t, formatNumber, useLocale as usePageLocale } from '../../i18n';
import { Button, ContextMenu, Icon, Input, Loading, Modal, Popover, cn } from '@dgos/ui';
import { SESSION_CAP, useChatStore, type ChatSession } from '../../stores/chatStore';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { ChatNotice } from './ChatNotice';

function SessionTab({ session, selected, tabIndex, running, dirty, onFocus, onSelect, onClose }: {
  session: ChatSession; selected: boolean; tabIndex: number; running: boolean; dirty: boolean;
  onFocus: () => void; onSelect: () => void; onClose: () => void;
}) {
  const rename = useChatStore(state => state.renameSession);
  const deleteSession = useChatStore(state => state.deleteSession);
  const [action, setAction] = useState<'rename' | 'delete' | null>(null);
  const [title, setTitle] = useState('');
  const [error, setError] = useState<string | null>(null);
  const tab = useRef<HTMLButtonElement>(null);
  const beginAction = (next: 'rename' | 'delete' | 'close') => {
    // Wait for the menu to close before Modal captures its return-focus target.
    requestAnimationFrame(() => {
      if (!tab.current) return;
      tab.current.focus({ preventScroll: true });
      if (next === 'close') { onClose(); return; }
      setTitle(session.title);
      setError(null);
      setAction(next);
    });
  };
  return <>
    <ContextMenu items={[
      { kind: 'item', id: 'rename', label: t('chat.rename'), onSelect: () => beginAction('rename') },
      { kind: 'item', id: 'close', label: t('chat.closeSession'), onSelect: () => beginAction('close') },
      { kind: 'separator' },
      { kind: 'item', id: 'delete', label: t('chat.delete'), danger: true, onSelect: () => beginAction('delete') },
    ]}>
      <div className={cn('workspace-tab', selected && 'is-active')}>
        <button ref={tab} type="button" role="tab" id={`chat-session-tab-${session.id}`} aria-controls={selected ? 'chat-session-panel' : undefined} tabIndex={tabIndex} aria-selected={selected} title={session.title} onFocus={onFocus} onClick={onSelect} onKeyDown={event => {
          if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return;
          event.preventDefault();
          const bounds = event.currentTarget.getBoundingClientRect();
          event.currentTarget.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: bounds.left, clientY: bounds.bottom }));
        }} className="workspace-tab-main">
          <span>{session.title}</span>
          {running ? <i className="workspace-running-dot" role="img" aria-label={t('chat.generating')} /> : null}
          {dirty ? <i className="workspace-dirty-dot" role="img" aria-label={t('shell.unsavedTitle')} /> : null}
        </button>
        <button type="button" aria-label={t('chat.closeTab', { title: session.title })} onClick={onClose} className="workspace-tab-close"><Icon name="close" size="sm" /></button>
      </div>
    </ContextMenu>
    <Modal open={action !== null} onOpenChange={open => { if (!open) { setAction(null); requestAnimationFrame(() => tab.current?.focus({ preventScroll: true })); } }} kind={action === 'rename' ? 'form' : 'confirm'} title={t(action === 'rename' ? 'chat.rename' : 'chat.delete')} icon={action === 'delete' ? 'warning' : undefined} description={action === 'delete' ? t('chat.deleteConfirm') : undefined} confirmText={t(action === 'rename' ? 'chat.saveTitle' : 'chat.delete')} confirmVariant={action === 'delete' ? 'danger' : 'primary'} onConfirm={async () => {
      const failure = action === 'rename' ? await rename(session.id, title) : await deleteSession(session.id);
      if (failure) { setError(failure); throw new Error('session_action_failed'); }
    }}>
      {action === 'rename' ? <Input aria-label={t('chat.title')} value={title} error={error ?? undefined} autoFocus onChange={event => setTitle(event.target.value)} /> : error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
    </Modal>
  </>;
}

export default function SessionSidebar({ onClose }: { onClose: (session: ChatSession) => void }) {
  usePageLocale();
  const active = useSurfaceActive();
  const sessions = useChatStore(state => state.sessions);
  const openIds = useChatStore(state => state.openSessionIds);
  const activeId = useChatStore(state => state.activeSessionId);
  const generations = useChatStore(state => state.generations);
  const drafts = useChatStore(state => state.sessionDrafts);
  const targetDrafts = useChatStore(state => state.sessionTargetDrafts);
  const createSession = useChatStore(state => state.createSession);
  const loadSessions = useChatStore(state => state.loadSessions);
  const loadMoreSessions = useChatStore(state => state.loadMoreSessions);
  const cursor = useChatStore(state => state.sessionsCursor);
  const loading = useChatStore(state => state.sessionsLoading);
  const loaded = useChatStore(state => state.sessionsLoaded);
  const error = useChatStore(state => state.sessionsError);
  const query = useChatStore(state => state.searchQuery);
  const results = useChatStore(state => state.searchResults);
  const searchError = useChatStore(state => state.searchError);
  const setQuery = useChatStore(state => state.setSearchQuery);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [overflowOpen, setOverflowOpen] = useState(false);
  const [roving, setRoving] = useState(activeId);
  const list = useRef<HTMLDivElement>(null);
  const previous = useRef(openIds);
  const newButton = useRef<HTMLButtonElement>(null);
  const open = openIds.map(id => sessions.find(session => session.id === id)).filter((session): session is ChatSession => !!session);
  const select = (session: ChatSession) => { void useChatStore.getState().selectSession(session); setHistoryOpen(false); setOverflowOpen(false); };
  const searching = query.trim().length > 0;
  useEffect(() => { if (!active) { setHistoryOpen(false); setOverflowOpen(false); } }, [active]);
  useEffect(() => {
    setRoving(activeId ?? openIds[0] ?? null);
    list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    const closed = previous.current.some(id => !openIds.includes(id));
    previous.current = openIds;
    if (closed && active) requestAnimationFrame(() => requestAnimationFrame(() => {
      (list.current?.querySelector<HTMLButtonElement>('[aria-selected="true"]') ?? newButton.current)?.focus();
    }));
  }, [activeId, openIds, active]);
  const row = (session: ChatSession) => <div key={session.id} className="workspace-tab-list-row"><button type="button" aria-current={session.id === activeId} title={session.title} onClick={() => select(session)}><span>{session.title}</span>{generations[session.id] ? <i className="workspace-running-dot" aria-label={t('chat.generating')} /> : null}{session.id === activeId ? <Icon name="check" size="sm" /> : null}</button><Button size="xxs" variant="ghost" iconOnly aria-label={t('chat.closeTab', { title: session.title })} onClick={() => { setOverflowOpen(false); onClose(session); }}><Icon name="close" size="sm" /></Button></div>;
  return <section aria-label={t('chat.innerTabs')} className="workspace-session-tabs">
    <div className="workspace-session-bar">
      <div ref={list} role="tablist" aria-label={t('chat.innerTabs')} className="workspace-tabs-scroll" onKeyDown={event => {
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
        const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
        const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
        if (index < 0) return;
        event.preventDefault();
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : -1) + buttons.length) % buttons.length;
        buttons[next]?.focus(); buttons[next]?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      }}>{open.map(session => <SessionTab key={session.id} session={session} selected={session.id === activeId} tabIndex={(roving ?? activeId ?? openIds[0]) === session.id ? 0 : -1} running={!!generations[session.id]} dirty={!!(drafts[session.id] || targetDrafts[session.id])} onFocus={() => setRoving(session.id)} onSelect={() => select(session)} onClose={() => onClose(session)} />)}</div>
      <Button ref={newButton} size="xxs" variant="ghost" iconOnly aria-label={t('chat.newSession')} onClick={() => void createSession()}><Icon name="plus" size="sm" /></Button>
      <Popover align="end" open={overflowOpen} onOpenChange={setOverflowOpen} label={t('chat.innerTabs')} trigger={<Button size="xxs" variant="ghost" iconOnly aria-label={t('chat.openTabs')}><Icon name="menu" size="sm" /></Button>} content={<div className="workspace-tab-list"><p className="workspace-menu-title">{t('chat.innerTabs')}</p>{open.map(row)}</div>} />
      <Popover align="end" open={historyOpen} onOpenChange={setHistoryOpen} label={t('chat.history')} trigger={<Button size="xxs" variant="ghost" iconOnly aria-label={t('chat.history')}><Icon name="chevron-down" size="sm" /></Button>} content={<div className="workspace-session-history"><Input aria-label={t('chat.search')} placeholder={t('chat.searchPlaceholder')} value={query} error={searchError ?? undefined} onChange={event => setQuery(event.target.value)} />
        {searching ? <div role="region" aria-label={t('chat.searchResults')} className="max-h-48 overflow-y-auto"><ul className="m-0 list-none p-0">{results.map(hit => <li key={hit.session.id}><button type="button" onClick={() => select(hit.session)} className="workspace-history-choice">{hit.session.title}<span>{formatNumber(hit.match_count)}</span></button></li>)}</ul></div> : <div role="region" aria-label={t('chat.sessionList')} aria-busy={loading} className="max-h-48 overflow-y-auto"><ul className="m-0 list-none p-0">{sessions.map(session => <li key={session.id}><button type="button" aria-current={session.id === activeId} onClick={() => select(session)} className="workspace-history-choice">{session.title}</button></li>)}</ul></div>}
        {error ? <div className="flex items-center gap-2"><ChatNotice tone="error">{error}</ChatNotice><Button size="sm" variant="secondary" onClick={() => void loadSessions()}>{t('chat.retry')}</Button></div> : null}
        {!searching && cursor && sessions.length < SESSION_CAP ? <Button size="sm" variant="secondary" loading={loading} onClick={() => void loadMoreSessions()}>{t('chat.moreSessions')}</Button> : null}
        {!searching && sessions.length >= SESSION_CAP ? <p className="m-0 text-xs text-ink-secondary">{t('chat.sessionCap', { count: formatNumber(SESSION_CAP) })}</p> : null}
      </div>} />
    </div>
    {!historyOpen && !open.length && loaded && !loading && !error ? <p className="m-0 text-sm text-ink-secondary">{t('chat.noSessions')}</p> : null}
    {loading && !sessions.length ? <Loading label={t('chat.loadingSessions')} /> : null}
  </section>;
}

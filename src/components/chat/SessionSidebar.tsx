import { t, formatNumber, useLocale as usePageLocale } from '../../i18n';
import { Button, Input, Loading, cn } from '@dgos/ui';
import { SESSION_CAP, useChatStore, type ChatSession } from '../../stores/chatStore';
import { ChatNotice } from './ChatNotice';
import { formatDateTime } from '../../i18n';

const formatTime = (seconds: number) => formatDateTime(seconds * 1000);

function SessionButton({ session, active, snippet, matches }: { session: ChatSession; active: boolean; snippet?: string; matches?: number }) {
  usePageLocale();
  const select = useChatStore(state => state.selectSession);
  return (
    <Button
      variant={active ? 'secondary' : 'ghost'}
      aria-current={active ? 'true' : undefined}
      className={cn('h-auto w-full flex-col items-start justify-start gap-1 whitespace-normal py-2 text-left')}
      onClick={() => void select(session)}
    >
      <span className="w-full font-medium wrap-anywhere">{session.title}</span>
      {snippet ? <span className="w-full text-xs font-normal text-muted-foreground wrap-anywhere">{snippet}</span> : null}
      <span className="text-xs font-normal text-muted-foreground">
        {formatTime(session.updated_at)}{matches !== undefined ? t('chat.matches', { count: formatNumber(matches) }) : ''}
      </span>
    </Button>
  );
}

export default function SessionSidebar() {
  usePageLocale();
  const sessions = useChatStore(state => state.sessions);
  const cursor = useChatStore(state => state.sessionsCursor);
  const capped = useChatStore(state => state.sessionsCapped);
  const loading = useChatStore(state => state.sessionsLoading);
  const loaded = useChatStore(state => state.sessionsLoaded);
  const error = useChatStore(state => state.sessionsError);
  const query = useChatStore(state => state.searchQuery);
  const results = useChatStore(state => state.searchResults);
  const searchCursor = useChatStore(state => state.searchCursor);
  const searchCapped = useChatStore(state => state.searchCapped);
  const searchLoading = useChatStore(state => state.searchLoading);
  const searchError = useChatStore(state => state.searchError);
  const activeId = useChatStore(state => state.activeSession?.id ?? null);
  const loadSessions = useChatStore(state => state.loadSessions);
  const loadMore = useChatStore(state => state.loadMoreSessions);
  const setQuery = useChatStore(state => state.setSearchQuery);
  const loadMoreSearch = useChatStore(state => state.loadMoreSearch);
  const createSession = useChatStore(state => state.createSession);
  const searching = query.trim().length > 0;

  return (
    <nav aria-label={t('chat.sessions')} className="flex h-full min-h-0 flex-col gap-3">
      <Button onClick={() => void createSession()}>{t('chat.newSession')}</Button>
      <Input
        aria-label={t('chat.search')}
        type="text"
        placeholder={t('chat.searchPlaceholder')}
        value={query}
        error={searchError ?? undefined}
        onChange={event => setQuery(event.target.value)}
      />
      <div className="min-h-0 grow overflow-y-auto">
        {searching ? (
          <section aria-label={t('chat.searchResults')} aria-busy={searchLoading}>
            {searchLoading && !results.length ? <Loading label={t('chat.searching')} /> : null}
            {!searchLoading && !searchError && !results.length ? <p className="m-0 text-sm text-muted-foreground">{t('chat.noMatches')}</p> : null}
            <ul className="m-0 flex list-none flex-col gap-1 p-0">
              {results.map(hit => (
                <li key={hit.session.id}><SessionButton session={hit.session} active={hit.session.id === activeId} snippet={hit.snippet} matches={hit.match_count} /></li>
              ))}
            </ul>
            {searchCursor && !searchCapped ? <Button variant="secondary" className="mt-2 w-full" loading={searchLoading} onClick={() => void loadMoreSearch()}>{t('chat.moreResults')}</Button> : null}
            {searchCapped ? <p className="mt-2 text-xs text-muted-foreground">{t('chat.resultCap', { count: formatNumber(SESSION_CAP) })}</p> : null}
          </section>
        ) : (
          <section aria-label={t('chat.sessionList')} aria-busy={loading}>
            {loading && !sessions.length ? <Loading label={t('chat.loadingSessions')} /> : null}
            {error ? (
              <div className="flex flex-col gap-2">
                <ChatNotice tone="error">{error}</ChatNotice>
                <Button variant="secondary" onClick={() => void loadSessions()}>{t('chat.retry')}</Button>
              </div>
            ) : null}
            {loaded && !loading && !error && !sessions.length ? <p className="m-0 text-sm text-muted-foreground">{t('chat.noSessions')}</p> : null}
            <ul className="m-0 flex list-none flex-col gap-1 p-0">
              {sessions.map(session => <li key={session.id}><SessionButton session={session} active={session.id === activeId} /></li>)}
            </ul>
            {cursor && !capped && !error ? <Button variant="secondary" className="mt-2 w-full" loading={loading} onClick={() => void loadMore()}>{t('chat.moreSessions')}</Button> : null}
            {capped ? <p className="mt-2 text-xs text-muted-foreground">{t('chat.sessionCap', { count: formatNumber(SESSION_CAP) })}</p> : null}
          </section>
        )}
      </div>
    </nav>
  );
}

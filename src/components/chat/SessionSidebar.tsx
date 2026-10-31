import { Button, Input, Loading, cn } from '@dgos/ui';
import { SESSION_CAP, useChatStore, type ChatSession } from '../../stores/chatStore';
import { ChatNotice } from './ChatNotice';

const formatTime = (seconds: number) => new Date(seconds * 1000).toLocaleString();

function SessionButton({ session, active, snippet, matches }: { session: ChatSession; active: boolean; snippet?: string; matches?: number }) {
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
        {formatTime(session.updated_at)}{matches !== undefined ? ` · 匹配 ${matches} 处` : ''}
      </span>
    </Button>
  );
}

export default function SessionSidebar() {
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
    <nav aria-label="会话" className="flex h-full min-h-0 flex-col gap-3">
      <Button onClick={() => void createSession()}>新建会话</Button>
      <Input
        aria-label="搜索会话"
        type="text"
        placeholder="搜索标题和消息内容"
        value={query}
        error={searchError ?? undefined}
        onChange={event => setQuery(event.target.value)}
      />
      <div className="min-h-0 grow overflow-y-auto">
        {searching ? (
          <section aria-label="搜索结果" aria-busy={searchLoading}>
            {searchLoading && !results.length ? <Loading label="正在搜索…" /> : null}
            {!searchLoading && !searchError && !results.length ? <p className="m-0 text-sm text-muted-foreground">没有匹配的会话。</p> : null}
            <ul className="m-0 flex list-none flex-col gap-1 p-0">
              {results.map(hit => (
                <li key={hit.session.id}><SessionButton session={hit.session} active={hit.session.id === activeId} snippet={hit.snippet} matches={hit.match_count} /></li>
              ))}
            </ul>
            {searchCursor && !searchCapped ? <Button variant="secondary" className="mt-2 w-full" loading={searchLoading} onClick={() => void loadMoreSearch()}>加载更多结果</Button> : null}
            {searchCapped ? <p className="mt-2 text-xs text-muted-foreground">已显示 {SESSION_CAP} 条结果，请缩小搜索词以查找更多。</p> : null}
          </section>
        ) : (
          <section aria-label="会话列表" aria-busy={loading}>
            {loading && !sessions.length ? <Loading label="正在加载会话…" /> : null}
            {error ? (
              <div className="flex flex-col gap-2">
                <ChatNotice tone="error">{error}</ChatNotice>
                <Button variant="secondary" onClick={() => void loadSessions()}>重试</Button>
              </div>
            ) : null}
            {loaded && !loading && !error && !sessions.length ? <p className="m-0 text-sm text-muted-foreground">还没有会话。点击“新建会话”开始。</p> : null}
            <ul className="m-0 flex list-none flex-col gap-1 p-0">
              {sessions.map(session => <li key={session.id}><SessionButton session={session} active={session.id === activeId} /></li>)}
            </ul>
            {cursor && !capped && !error ? <Button variant="secondary" className="mt-2 w-full" loading={loading} onClick={() => void loadMore()}>加载更多会话</Button> : null}
            {capped ? <p className="mt-2 text-xs text-muted-foreground">已显示最近 {SESSION_CAP} 个会话，请使用搜索查找更早的会话。</p> : null}
          </section>
        )}
      </div>
    </nav>
  );
}

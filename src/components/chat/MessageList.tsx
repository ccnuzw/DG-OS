import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Button, Loading, cn } from '@dgos/ui';
import { generationErrorText, useChatStore, type ChatMessage, type ChatToolProposal, type Generation } from '../../stores/chatStore';
import { ChatNotice, FOCUS } from './ChatNotice';
import { COPY_TEXT, useCopyFeedback } from './clipboard';
import Markdown from './Markdown';

const LONG_CONTENT_CHARS = 4000;
const ROLE_LABEL = { user: '用户', assistant: '助手' } as const;

/** The category string of a persisted message (`provider_1001006`, `task_cancelled`, ...) as readable text. */
function messageErrorText(code: string | null): string | null {
  if (!code) return null;
  const provider = /^provider_(\d+)$/.exec(code);
  return generationErrorText(code, provider ? Number(provider[1]) : null);
}

function Badge({ tone, children }: { tone: 'warning' | 'error' | 'info'; children: string }) {
  const classes = { warning: 'bg-warning-bg text-warning-text', error: 'bg-error-bg text-error-text', info: 'bg-info-bg text-info-text' };
  return <span className={cn('rounded-sm px-2 py-0.5 text-xs font-medium', classes[tone])}>{children}</span>;
}

function MessageItem({ message, isLast, canRegenerate, dropped }: { message: ChatMessage; isLast: boolean; canRegenerate: boolean; dropped: number }) {
  const regenerate = useChatStore(state => state.regenerate);
  const [copyState, copy] = useCopyFeedback();
  const [expanded, setExpanded] = useState(false);
  const assistant = message.role === 'assistant';
  const long = message.content.length > LONG_CONTENT_CHARS;
  const collapsed = long && !expanded;
  const errorText = message.status === 'completed' ? null : messageErrorText(message.error_code);
  return (
    <li>
      <article aria-label={`${ROLE_LABEL[message.role]}消息`} className={cn('rounded-lg border border-border p-3', assistant ? 'bg-card' : 'bg-background')}>
        <header className="mb-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <strong className="text-sm text-foreground">{ROLE_LABEL[message.role]}</strong>
          {message.status === 'interrupted' ? <Badge tone="warning">已中断</Badge> : null}
          {message.status === 'failed' ? <Badge tone="error">生成失败</Badge> : null}
          {message.status === 'generating' ? <Badge tone="info">生成中</Badge> : null}
        </header>
        <div className={cn(collapsed && 'max-h-96 overflow-y-auto')} tabIndex={collapsed ? 0 : undefined} aria-label={collapsed ? '长内容，可滚动' : undefined}>
          {assistant
            ? (message.content ? <Markdown source={message.content} /> : <p className="m-0 text-sm text-muted-foreground">（没有内容）</p>)
            : <p className="m-0 whitespace-pre-wrap text-sm wrap-anywhere">{message.content}</p>}
        </div>
        {long ? <Button size="sm" variant="ghost" className="mt-1" aria-expanded={expanded} onClick={() => setExpanded(value => !value)}>{expanded ? '收起长内容' : '展开全部'}</Button> : null}
        {assistant && dropped > 0 ? <p className="mt-2 mb-0 text-xs text-muted-foreground">较早的完整轮次未发送给模型（已省略 {dropped} 轮）。</p> : null}
        {assistant && message.finish_reason === 'length' ? <p className="mt-2 mb-0 text-xs text-warning-text">回复因达到长度上限被截断。</p> : null}
        {errorText ? <p className="mt-2 mb-0 text-xs text-error-text">{errorText}</p> : null}
        {assistant && message.result_unknown ? <p className="mt-2 mb-0 text-xs text-warning-text">结果未知，上游可能已计费。</p> : null}
        {assistant && message.status !== 'generating' ? (
          <footer className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span>{message.total_tokens === null ? '用量未知' : `用量 ${message.total_tokens} tokens`}</span>
            <Button size="sm" variant="ghost" onClick={() => copy(message.content)} disabled={!message.content}>复制</Button>
            {isLast && canRegenerate ? <Button size="sm" variant="ghost" onClick={() => void regenerate(message.id)}>重新生成</Button> : null}
            <span role="status" className={copyState === 'failed' ? 'text-error-text' : 'text-success-text'}>{COPY_TEXT[copyState]}</span>
          </footer>
        ) : null}
        {!assistant ? (
          <footer className="mt-2 flex items-center gap-2 text-xs text-muted-foreground">
            <Button size="sm" variant="ghost" onClick={() => copy(message.content)}>复制</Button>
            <span role="status" className={copyState === 'failed' ? 'text-error-text' : 'text-success-text'}>{COPY_TEXT[copyState]}</span>
          </footer>
        ) : null}
      </article>
    </li>
  );
}

function ToolProposalCard({ proposal, busy }: { proposal: ChatToolProposal; busy: boolean }) {
  const decide = useChatStore(state => state.decideTool);
  const heading = useRef<HTMLHeadingElement>(null);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    setNow(Date.now());
    heading.current?.focus();
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [proposal.id]);
  const remaining = Math.max(0, Math.ceil((proposal.expires_at * 1000 - now) / 1000));
  const expired = remaining === 0;
  return (
    <section aria-labelledby="chat-proposal-heading" className="rounded-lg border-2 border-primary bg-card p-3">
      <h3 id="chat-proposal-heading" ref={heading} tabIndex={-1} className={cn('m-0 mb-2 text-base font-semibold', FOCUS)}>确认工具调用</h3>
      <p className="m-0 mb-2 text-sm wrap-anywhere">
        模型请求调用 <strong>{proposal.server_name} / {proposal.tool_name}</strong>。请核对下面的完整参数；工具可能修改数据或把数据发送到外部。
      </p>
      <pre tabIndex={0} aria-label="工具调用参数" className={cn('m-0 mb-2 overflow-x-auto rounded-md bg-secondary p-3 font-mono text-xs whitespace-pre-wrap wrap-anywhere', FOCUS)}>
        {JSON.stringify(proposal.arguments, null, 2)}
      </pre>
      <p className="m-0 mb-2 text-xs text-muted-foreground">{expired ? '提案已过期，无法再批准。' : `剩余 ${remaining} 秒。批准只对这一次调用有效，不适用于以后的调用。`}</p>
      <div className="flex flex-wrap gap-2">
        <Button disabled={busy || expired} onClick={() => void decide('approve')}>批准本次调用</Button>
        <Button variant="secondary" disabled={busy || expired} onClick={() => void decide('deny')}>拒绝并停止</Button>
      </div>
    </section>
  );
}

const PHASE_TEXT: Readonly<Record<Generation['phase'], string>> = {
  sending: '正在发送…',
  running: '正在生成…',
  awaiting_tool: '等待你确认工具调用…',
  tool_running: '正在执行工具并等待模型…',
};

function StreamingBubble({ generation }: { generation: Generation }) {
  const [showReasoning, setShowReasoning] = useState(false);
  const status = generation.stopping ? '正在停止…' : generation.toolsMode && generation.phase === 'running' ? '带工具的轮次不流式，正在等待模型…' : PHASE_TEXT[generation.phase];
  return (
    <li>
      <article aria-label="助手消息（生成中）" aria-busy="true" className="rounded-lg border border-border bg-card p-3">
        <header className="mb-2 flex flex-wrap items-center gap-2">
          <strong className="text-sm">助手</strong>
          <Loading size="sm" label={status} />
        </header>
        {generation.text ? <Markdown source={generation.text} /> : null}
        {generation.reasoning ? (
          <div className="mt-2">
            <Button size="sm" variant="ghost" aria-expanded={showReasoning} onClick={() => setShowReasoning(value => !value)}>{showReasoning ? '收起思考过程' : '展开思考过程'}</Button>
            {showReasoning ? (
              <p aria-label="思考过程（仅显示，不保存）" className="m-0 mt-1 whitespace-pre-wrap rounded-md bg-secondary p-2 text-xs text-muted-foreground wrap-anywhere">{generation.reasoning}</p>
            ) : null}
          </div>
        ) : null}
        {generation.proposal ? <div className="mt-3"><ToolProposalCard proposal={generation.proposal} busy={generation.proposalBusy} /></div> : null}
      </article>
    </li>
  );
}

export default function MessageList() {
  const messages = useChatStore(state => state.messages);
  const generation = useChatStore(state => state.generation);
  const activeId = useChatStore(state => state.activeSession?.id ?? null);
  const olderCursor = useChatStore(state => state.olderCursor);
  const trimmed = useChatStore(state => state.newerTrimmed);
  const loading = useChatStore(state => state.messagesLoading);
  const error = useChatStore(state => state.messagesError);
  const stale = useChatStore(state => state.activeStale);
  const dropped = useChatStore(state => state.droppedTurns);
  const loadOlder = useChatStore(state => state.loadOlderMessages);
  const loadLatest = useChatStore(state => state.loadLatestMessages);

  const live = generation && generation.sessionId === activeId ? generation : null;
  const hidden = live?.replacesMessageId ?? null;
  // The reply that is being generated is shown by the bubble, not by a persisted placeholder.
  const visible = messages.filter(message => message.id !== hidden && message.id !== live?.assistantMessageId);
  const last = visible[visible.length - 1];

  const container = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const previousHeight = useRef<number | null>(null);
  const firstId = visible[0]?.id;

  useLayoutEffect(() => {
    const element = container.current;
    if (!element) return;
    if (previousHeight.current !== null) {
      // Older messages were prepended: keep the same message under the pointer.
      element.scrollTop += element.scrollHeight - previousHeight.current;
      previousHeight.current = null;
    } else if (stick.current) {
      element.scrollTop = element.scrollHeight;
    }
  }, [firstId, visible.length, live?.text, live?.phase, live?.proposal?.id]);

  const onScroll = () => {
    const element = container.current;
    if (element) stick.current = element.scrollHeight - element.scrollTop - element.clientHeight < 48;
  };

  return (
    <div ref={container} onScroll={onScroll} className="min-h-0 grow overflow-y-auto" aria-busy={loading}>
      <div className="flex flex-col gap-3 p-1">
        {olderCursor !== null ? (
          <Button variant="secondary" loading={loading} onClick={() => { previousHeight.current = container.current?.scrollHeight ?? null; void loadOlder(); }}>加载更早的消息</Button>
        ) : null}
        {loading && !visible.length ? <Loading label="正在加载消息…" /> : null}
        {error ? (
          <div className="flex flex-col gap-2">
            <ChatNotice tone="error">{error}</ChatNotice>
            {!stale ? <Button variant="secondary" onClick={() => void loadLatest()}>重试</Button> : null}
          </div>
        ) : null}
        {!loading && !error && !visible.length && !live ? <p className="m-0 text-sm text-muted-foreground">这个会话还没有消息。在下方输入内容开始对话。</p> : null}
        <ol aria-label="消息" className="m-0 flex list-none flex-col gap-3 p-0">
          {visible.map(message => (
            <MessageItem key={message.id} message={message} isLast={message.id === last?.id}
              canRegenerate={!live && !trimmed && !stale} dropped={dropped[message.id] ?? 0} />
          ))}
          {live ? <StreamingBubble generation={live} /> : null}
        </ol>
        {trimmed ? (
          <div className="flex flex-col gap-1">
            <p className="m-0 text-xs text-muted-foreground">为控制页面规模，较新的消息已从当前窗口移除。</p>
            <Button variant="secondary" onClick={() => void loadLatest()}>回到最新</Button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

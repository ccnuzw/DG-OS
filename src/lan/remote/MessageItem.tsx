import { Button } from '@dgos/ui/components/Button';
import { cn } from '@dgos/ui/lib/cn';
import type { ChatMessage } from './api';
import { displayContent, type FailureInfo, type LiveText } from './chatState';
import { failedMessageText } from './messages';

export interface MessageItemProps {
  message: ChatMessage;
  live: LiveText | undefined;
  failure: FailureInfo | undefined;
  canRegenerate: boolean;
  onRegenerate: (message: ChatMessage) => void;
  onCancel: (taskId: string) => void;
}

/** One chat message. Content is rendered as plain text (pre-wrap): never as HTML or markdown. */
export function MessageItem({ message, live, failure, canRegenerate, onRegenerate, onCancel }: MessageItemProps) {
  const mine = message.role === 'user';
  const generating = message.status === 'generating';
  const content = displayContent(message, live);
  const tokens = typeof message.total_tokens === 'number' && message.total_tokens > 0 ? message.total_tokens : null;
  const taskId = message.task_id;
  const unknownResult = failure?.resultUnknown ?? message.result_unknown;

  return (
    <li className={cn('flex flex-col gap-1', mine ? 'items-end' : 'items-start')} data-role={message.role}>
      <div
        className={cn(
          'max-w-full rounded-lg px-3 py-2 text-base',
          mine ? 'bg-primary text-on-primary' : 'bg-secondary text-secondary-foreground',
        )}
      >
        {generating && live?.reasoning ? (
          <details className="mb-2 text-sm text-muted-foreground">
            <summary>思考过程</summary>
            <p className="whitespace-pre-wrap break-words">{live.reasoning}</p>
          </details>
        ) : null}
        {content ? <p className="whitespace-pre-wrap break-words">{content}</p> : null}
        {generating && !content ? <p className="text-muted-foreground">正在思考…</p> : null}
      </div>
      <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
        {generating ? <span>生成中…</span> : null}
        {generating && taskId ? (
          <Button size="sm" variant="secondary" onClick={() => onCancel(taskId)}>
            停止生成
          </Button>
        ) : null}
        {message.status === 'interrupted' || message.status === 'failed' ? (
          <span role="status" className="text-error-text">
            {failedMessageText(message.status, failure?.errorCode ?? message.error_code, failure?.providerCode ?? null)}
          </span>
        ) : null}
        {unknownResult && !generating ? <span>结果未知，上游可能已产生用量</span> : null}
        {tokens !== null ? <span>{tokens} tokens</span> : null}
        {canRegenerate ? (
          <Button size="sm" variant="ghost" onClick={() => onRegenerate(message)}>
            重新生成
          </Button>
        ) : null}
      </div>
    </li>
  );
}

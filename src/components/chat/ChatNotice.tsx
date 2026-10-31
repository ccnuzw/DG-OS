import type { ReactNode } from 'react';
import { Button, StatusIcon, cn } from '@dgos/ui';

export const FOCUS = 'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus-ring';

const TONE: Readonly<Record<'error' | 'warning' | 'info', string>> = {
  error: 'border-error bg-error-bg text-error-text',
  warning: 'border-warning bg-warning-bg text-warning-text',
  info: 'border-info bg-info-bg text-info-text',
};

export interface ChatNoticeProps {
  tone: 'error' | 'warning' | 'info';
  children: ReactNode;
  onDismiss?: () => void;
  className?: string;
}

/** Inline notice. Errors use role="alert"; warnings and information use role="status" so they are announced politely. */
export function ChatNotice({ tone, children, onDismiss, className }: ChatNoticeProps) {
  return (
    <div role={tone === 'error' ? 'alert' : 'status'} className={cn('flex items-start gap-2 rounded-md border-l-4 p-3 text-sm', TONE[tone], className)}>
      <StatusIcon name={tone === 'info' ? 'info' : tone === 'error' ? 'error' : 'warning'} className="mt-0.5" />
      <p className="m-0 min-w-0 grow wrap-anywhere">{children}</p>
      {onDismiss ? <Button size="sm" variant="ghost" onClick={onDismiss}>知道了</Button> : null}
    </div>
  );
}

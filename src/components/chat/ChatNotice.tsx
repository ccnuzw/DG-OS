import { t, useLocale as usePageLocale } from '../../i18n';
import type { ReactNode } from 'react';
import { Button, StatusIcon, cn } from '@dgos/ui';

export const FOCUS = 'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-border-focus';

const TONE: Readonly<Record<'error' | 'warning' | 'info', string>> = {
  error: 'border-status-error bg-status-error-surface text-status-error-text',
  warning: 'border-status-warning bg-status-warning-surface text-status-warning-text',
  info: 'border-status-info bg-status-info-surface text-status-info-text',
};

export interface ChatNoticeProps {
  tone: 'error' | 'warning' | 'info';
  children: ReactNode;
  onDismiss?: () => void;
  className?: string;
}

/** Inline notice. Errors use role="alert"; warnings and information use role="status" so they are announced politely. */
export function ChatNotice({ tone, children, onDismiss, className }: ChatNoticeProps) {
  usePageLocale();
  return (
    <div role={tone === 'error' ? 'alert' : 'status'} className={cn('flex items-start gap-2 rounded-md border-l-4 p-3 text-sm', TONE[tone], className)}>
      <StatusIcon name={tone === 'info' ? 'info' : tone === 'error' ? 'error' : 'warning'} className="mt-0.5" />
      <p className="m-0 min-w-0 grow wrap-anywhere">{children}</p>
      {onDismiss ? <Button size="sm" variant="ghost" onClick={onDismiss}>{t('chat.acknowledge')}</Button> : null}
    </div>
  );
}

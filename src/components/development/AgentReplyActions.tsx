import { useRef } from 'react';
import { Button, DropdownMenu } from '@dgos/ui';
import { Copy, Ellipsis, Quote } from 'lucide-react';
import { useLocale } from '../../i18n';
import { COPY_TEXT, useCopyFeedback } from '../chat/clipboard';

export function AgentReplyActions({ source, onQuote, compact = false }: { source: string; onQuote?: (text: string) => void; compact?: boolean }) {
  const { t } = useLocale();
  const [state, copy] = useCopyFeedback();
  const quoteAfterClose = useRef<(() => void) | null>(null);
  return <div className="agent-reply-actions" data-compact={compact || undefined} role="group" aria-label={t('agentReply.actions')}>
    {compact ? <DropdownMenu label={t('agentReply.actions')} onCloseAutoFocus={event => {
      const quote = quoteAfterClose.current;
      quoteAfterClose.current = null;
      if (quote) { event.preventDefault(); quote(); }
    }} trigger={<Button size="xs" variant="ghost" iconOnly aria-label={t('agentReply.actions')} title={t('agentReply.actions')}><Ellipsis size={15} aria-hidden="true" /></Button>} items={[
      { kind: 'item', id: 'copy', label: t('agentReply.copy'), icon: 'copy', onSelect: () => copy(source) },
      ...(onQuote ? [{ kind: 'item' as const, id: 'quote', label: t('agentReply.quote'), onSelect: () => { quoteAfterClose.current = () => onQuote(source); } }] : []),
    ]} /> : <>
      <Button size="xs" variant="ghost" onClick={() => copy(source)}><Copy size={13} aria-hidden="true" />{t('agentReply.copy')}</Button>
      {onQuote ? <Button size="xs" variant="ghost" onClick={() => onQuote(source)}><Quote size={13} aria-hidden="true" />{t('agentReply.quote')}</Button> : null}
    </>}
    <span role="status" className="agent-reply-note">{COPY_TEXT[state]}</span>
  </div>;
}

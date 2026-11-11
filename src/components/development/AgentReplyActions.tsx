import { Button } from '@dgos/ui';
import { Copy, Quote } from 'lucide-react';
import { useLocale } from '../../i18n';
import { COPY_TEXT, useCopyFeedback } from '../chat/clipboard';

export function AgentReplyActions({ source, onQuote }: { source: string; onQuote?: (text: string) => void }) {
  const { t } = useLocale();
  const [state, copy] = useCopyFeedback();
  return <div className="agent-reply-actions" role="group" aria-label={t('agentReply.actions')}>
    <Button size="xs" variant="ghost" onClick={() => copy(source)}><Copy size={13} aria-hidden="true" />{t('agentReply.copy')}</Button>
    {onQuote ? <Button size="xs" variant="ghost" onClick={() => onQuote(source)}><Quote size={13} aria-hidden="true" />{t('agentReply.quote')}</Button> : null}
    <span role="status" className="agent-reply-note">{COPY_TEXT[state]}</span>
  </div>;
}

import { useMemo, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { CodeBlock } from '../chat/Markdown';
import { COPY_TEXT, useCopyFeedback } from '../chat/clipboard';

const PREVIEW_CHARS = 24000;
/** Search and copy operate on the complete native output; expensive highlighting stays bounded. */
export function AgentOutputText({ text, lang }: { text: string; lang?: string }) {
  const { t } = useLocale();
  const [query, setQuery] = useState('');
  const [expanded, setExpanded] = useState(false);
  const [state, copy] = useCopyFeedback();
  const large = text.length > PREVIEW_CHARS;
  const matched = useMemo(() => query ? text.split('\n').filter(line => line.toLocaleLowerCase().includes(query.toLocaleLowerCase())) : null, [text, query]);
  const source = matched ? matched.join('\n') : text;
  const truncated = source.length > PREVIEW_CHARS && !expanded;
  if (!large) return <CodeBlock text={text} lang={lang} />;
  return <div className="agent-reply-output">
    <div className="agent-reply-output-toolbar"><Input label={t('agentReply.searchOutput')} value={query} onChange={event => { setQuery(event.target.value); setExpanded(false); }} /><Button size="xs" variant="secondary" onClick={() => copy(text)}>{t('agentReply.copyOutput')}</Button><span role="status">{COPY_TEXT[state]}</span></div>
    {matched ? <p role="status" className="agent-reply-note">{t(matched.length ? 'agentReply.matchCount' : 'agentReply.noMatch', { count: matched.length })}</p> : null}
    {expanded ? <pre className="agent-reply-native-value type-code" tabIndex={0}>{source}</pre> : <CodeBlock text={source.slice(0, PREVIEW_CHARS)} lang={lang} />}
    {truncated ? <><p className="agent-reply-note">{t('agentReply.outputLimit', { count: PREVIEW_CHARS })}</p><Button size="xs" variant="ghost" onClick={() => setExpanded(true)}>{t('agentReply.expandOutput')}</Button></> : null}
  </div>;
}

import { useId, useMemo, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import { COPY_TEXT, useCopyFeedback } from '../chat/clipboard';
import './agent-reply-timeline.css';

const PREVIEW_LINES = 8;
const PREVIEW_CHARS = 1600;
const MAX_DISPLAY_CHARS = 24000;
const MAX_SEARCH_LINE_CHARS = 400;

function lineStarts(text: string): number[] {
  const starts = [0];
  for (let index = text.indexOf('\n'); index !== -1; index = text.indexOf('\n', index + 1)) starts.push(index + 1);
  return starts;
}

function codePointBoundary(text: string, index: number, direction: -1 | 1): number {
  const boundary = Math.max(0, Math.min(index, text.length));
  return boundary > 0 && boundary < text.length && /[\uD800-\uDBFF]/.test(text[boundary - 1]) && /[\uDC00-\uDFFF]/.test(text[boundary]) ? boundary + direction : boundary;
}

function literalPattern(query: string) {
  const chunks: { pattern: RegExp; offset: number }[] = [];
  for (let offset = 0; offset < query.length;) {
    const end = codePointBoundary(query, Math.min(query.length, offset + 512), -1);
    const source = query.slice(offset, end).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    chunks.push({ pattern: new RegExp(source, chunks.length ? 'iyu' : 'igu'), offset });
    offset = end;
  }
  return chunks;
}

function* literalMatches(source: string, chunks: ReturnType<typeof literalPattern>): Generator<{ index: number; length: number }> {
  const first = chunks[0]?.pattern;
  if (!first) return;
  first.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = first.exec(source))) {
    let length = match[0].length, complete = true;
    for (let index = 1; index < chunks.length; index++) {
      const chunk = chunks[index];
      chunk.pattern.lastIndex = match.index + chunk.offset;
      const next = chunk.pattern.exec(source);
      if (!next) { complete = false; break; }
      length += next[0].length;
    }
    if (complete) { yield { index: match.index, length }; first.lastIndex = match.index + length; }
    else first.lastIndex = match.index + (source.codePointAt(match.index)! > 0xffff ? 2 : 1);
  }
}

/** Full-output search and copy stay independent of the bounded display window. */
export function AgentOutputText({ text, lang }: { text: string; lang?: string }) {
  const { t } = useLocale();
  const id = useId();
  const output = useRef<HTMLPreElement>(null);
  const [query, setQuery] = useState('');
  const [expanded, setExpanded] = useState(false);
  const [located, setLocated] = useState(false);
  const [matchIndex, setMatchIndex] = useState(0);
  const [previousText, setPreviousText] = useState(text);
  // Append-only streaming keeps the reader's place; replacement snapshots do not.
  if (previousText !== text) {
    setPreviousText(text);
    if (!text.startsWith(previousText)) {
      setExpanded(false);
      setLocated(false);
      setMatchIndex(0);
    }
  }
  const [state, copy] = useCopyFeedback();
  const starts = useMemo(() => lineStarts(text), [text]);
  const pattern = useMemo(() => query ? literalPattern(query) : null, [query]);
  const matches = useMemo(() => {
    if (!pattern) return null;
    const found = [];
    for (let index = 0; index < starts.length; index++) {
      const start = starts[index], end = index + 1 < starts.length ? starts[index + 1] - 1 : text.length;
      const match = literalMatches(text.slice(start, end), pattern).next().value;
      if (match) found.push({ line: index + 1, start, end, column: match.index });
    }
    return found;
  }, [text, starts, pattern]);
  const previewEnd = codePointBoundary(text, Math.min(text.length, PREVIEW_CHARS, starts[PREVIEW_LINES] === undefined ? text.length : starts[PREVIEW_LINES] - 1), -1);
  const hasMore = previewEnd < text.length;
  const currentMatchIndex = Math.min(matchIndex, Math.max(0, (matches?.length ?? 0) - 1));
  const match = matches?.[currentMatchIndex];
  const location = located && match ? (() => {
    const matchStart = match.start + match.column;
    const context = Math.min(PREVIEW_CHARS / 2, Math.max(0, MAX_DISPLAY_CHARS - query.length));
    const start = codePointBoundary(text, Math.max(match.start, matchStart - context), 1);
    const nextLine = starts[match.line - 1 + PREVIEW_LINES];
    const endByLine = nextLine === undefined ? text.length : nextLine - 1;
    return { start, end: codePointBoundary(text, Math.min(endByLine, Math.max(start + PREVIEW_CHARS, matchStart + query.length), start + MAX_DISPLAY_CHARS), -1) };
  })() : null;
  const windowStart = location?.start ?? 0;
  const windowEnd = location?.end ?? (expanded ? codePointBoundary(text, Math.min(text.length, MAX_DISPLAY_CHARS), -1) : previewEnd);
  const source = text.slice(windowStart, windowEnd);
  const visibleLines = source ? source.split('\n').length : 0;
  const locate = (index: number) => {
    if (!matches?.[index]) return;
    setMatchIndex(index);
    setLocated(true);
    setExpanded(false);
    output.current?.focus({ preventScroll: true });
  };
  const activeMatchStart = location && match ? match.start + match.column - windowStart : -1;
  const highlighted = useMemo(() => {
    if (!pattern) return source;
    const nodes = [];
    let start = 0;
    if (activeMatchStart >= 0 && activeMatchStart + query.length > source.length) {
      nodes.push(source.slice(0, activeMatchStart), <mark key={activeMatchStart}>{source.slice(activeMatchStart)}</mark>);
      return nodes;
    }
    for (const match of literalMatches(source, pattern)) {
      nodes.push(source.slice(start, match.index), <mark key={match.index}>{source.slice(match.index, match.index + match.length)}</mark>);
      start = match.index + match.length;
    }
    nodes.push(source.slice(start));
    return nodes;
  }, [source, pattern, activeMatchStart, query.length]);
  const firstLine = useMemo(() => {
    let line = 0;
    while (line + 1 < starts.length && starts[line + 1] <= windowStart) line++;
    return line + 1;
  }, [starts, windowStart]);
  return <div className="agent-reply-output">
    <div className="agent-reply-output-toolbar">
      {hasMore || query ? <Input label={t('agentReply.searchOutput')} value={query} onChange={event => { setQuery(event.target.value); setMatchIndex(0); setLocated(false); setExpanded(false); }} /> : null}
      <Button size="xs" variant="secondary" onClick={() => copy(text)}>{t('agentReply.copyOutput')}</Button><span role="status">{COPY_TEXT[state]}</span>
    </div>
    {matches ? <div className="agent-reply-output-search">
      <p role="status" className="agent-reply-note">{t(matches.length ? 'agentReply.matchCount' : 'agentReply.noMatch', { count: matches.length })}</p>
      {match ? <>
        <div className="agent-reply-output-navigation"><span>{t('agentReply.outputMatchPosition', { index: currentMatchIndex + 1, count: matches.length })}</span><Button size="xs" variant="ghost" disabled={currentMatchIndex === 0} onClick={() => locate(currentMatchIndex - 1)}>{t('agentReply.previousOutputMatch')}</Button><Button size="xs" variant="ghost" disabled={currentMatchIndex >= matches.length - 1} onClick={() => locate(currentMatchIndex + 1)}>{t('agentReply.nextOutputMatch')}</Button></div>
        <Button className="agent-reply-output-match" size="xs" variant="ghost" onClick={() => locate(currentMatchIndex)}><span className="agent-reply-output-line-number">{t('agentReply.outputLocation', { start: match.line, end: match.line })}</span><code>{text.slice(codePointBoundary(text, Math.max(match.start, match.start + match.column - MAX_SEARCH_LINE_CHARS / 2), 1), codePointBoundary(text, Math.min(match.end, match.start + match.column + MAX_SEARCH_LINE_CHARS / 2), -1))}</code></Button>
      </> : null}
    </div> : null}
    {location ? <p className="agent-reply-note">{t('agentReply.outputLocation', { start: firstLine, end: firstLine + Math.max(0, visibleLines - 1) })}</p> : null}
    <pre id={id} ref={output} className="chat-code agent-reply-native-value agent-reply-output-preview type-code" tabIndex={0} aria-label={t('agentExecution.output')} data-output-language={lang} data-output-start-line={firstLine}><code>{highlighted}</code></pre>
    {hasMore ? <>
      <p className="agent-reply-note">{t('agentReply.outputPreview', { lines: visibleLines, chars: source.length })}</p>
      {windowEnd < text.length && source.length >= MAX_DISPLAY_CHARS - 1 ? <p className="agent-reply-note">{t('agentReply.outputSafetyLimit', { count: MAX_DISPLAY_CHARS })}</p> : null}
      <Button size="xs" variant="ghost" aria-expanded={expanded || Boolean(location)} aria-controls={id} onClick={() => { setLocated(false); setExpanded(value => location ? false : !value); }}>{t(expanded || location ? 'agentReply.collapseOutput' : 'agentReply.expandOutput')}</Button>
    </> : null}
  </div>;
}

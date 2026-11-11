import { useEffect, useMemo, useRef, useState } from 'react';
import { Button, Input, Select } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { AgentEvent } from '../../stores/agentRunnerStore';
import { object, text } from './agentTranscript';
import { inspectableAgentEvent } from './codexTranscript';
import './agent-reply.css';
import './agent-transcript-diagnostics.css';

const PAGE_SIZE = 50;
type EventFocus = { runId: string; seq: number; nonce: number };
type EventLocation = { runId: string; seq: number };
type LocateIssue = { kind: 'notFound' | 'ambiguous'; seq: number } | { kind: 'required' };

const eventRun = (event: AgentEvent) => event.run ?? '';
const eventType = (event: AgentEvent) => JSON.stringify([event.kind, text(object(event.data).native_type)]);

function DiagnosticEvent({ event, highlighted, focusToken }: { event: AgentEvent; highlighted: boolean; focusToken?: object }) {
  const { t } = useLocale();
  const [expanded, setExpanded] = useState(highlighted);
  const row = useRef<HTMLDetailsElement>(null);
  const nativeType = text(object(event.data).native_type);
  useEffect(() => {
    if (!highlighted || !focusToken) return;
    setExpanded(true);
    row.current?.querySelector('summary')?.focus({ preventScroll: true });
    row.current?.scrollIntoView?.({ block: 'nearest' });
  }, [highlighted, focusToken]);
  return <details ref={row} className="agent-reply-disclosure agent-reply-native agent-transcript-diagnostic-event" open={expanded} onToggle={event => setExpanded(event.currentTarget.open)} data-diagnostic-event data-event-run={eventRun(event)} data-event-seq={event.seq} data-focused-diagnostic={highlighted || undefined}>
    <summary><span className="agent-transcript-diagnostic-sequence">#{event.seq}</span><code>{event.kind}{nativeType ? ` / ${nativeType}` : event.kind === 'native/unsupported' ? ` / ${t('agentReply.diagnosticUnknownType')}` : ''}</code><span className="agent-transcript-diagnostic-run">{event.run || t('agentReply.diagnosticNoRun')}</span><span>{t('agent.details')}</span></summary>
    {expanded ? <div className="agent-reply-detail-body"><pre className="type-code agent-reply-native-value" tabIndex={0} data-event-seq={event.seq} data-event-run={eventRun(event)}>{JSON.stringify(inspectableAgentEvent(event), null, 2)}</pre></div> : null}
  </details>;
}

export function AgentTranscriptDiagnostics({ events, focusEvent }: { events: readonly AgentEvent[]; focusEvent?: EventFocus }) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const [selectedType, setSelectedType] = useState<string | null>(null);
  const [page, setPage] = useState(0);
  const [sequence, setSequence] = useState('');
  const [selectedRun, setSelectedRun] = useState<string | null>(null);
  const [located, setLocated] = useState<EventLocation>();
  const [focusToken, setFocusToken] = useState<object>();
  const [issue, setIssue] = useState<LocateIssue>();
  const externalFocus = useRef<{ request: EventFocus; found: boolean } | undefined>(undefined);
  const { groups, runs } = useMemo(() => {
    const groups = new Map<string, { key: string; kind: string; nativeType: string; count: number }>();
    const runs = new Set<string>();
    for (const event of events) {
      const key = eventType(event), group = groups.get(key);
      if (group) group.count++;
      else groups.set(key, { key, kind: event.kind, nativeType: text(object(event.data).native_type), count: 1 });
      runs.add(eventRun(event));
    }
    return { groups: [...groups.values()], runs: [...runs] };
  }, [events]);
  const filtered = useMemo(() => selectedType === null ? events : events.filter(event => eventType(event) === selectedType), [events, selectedType]);
  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(page, pages - 1);
  const pageEvents = filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE);

  useEffect(() => {
    if (!focusEvent) return;
    const previous = externalFocus.current;
    const request = previous?.request;
    const repeated = request?.runId === focusEvent.runId && request.seq === focusEvent.seq && request.nonce === focusEvent.nonce;
    const index = events.findIndex(event => eventRun(event) === focusEvent.runId && event.seq === focusEvent.seq);
    if (repeated && previous?.found === (index >= 0)) return;
    externalFocus.current = { request: { ...focusEvent }, found: index >= 0 };
    setOpen(true);
    setSelectedType(null);
    setSequence(String(focusEvent.seq));
    setSelectedRun(focusEvent.runId);
    setIssue(index < 0 ? { kind: 'notFound', seq: focusEvent.seq } : undefined);
    setLocated(index >= 0 ? { runId: focusEvent.runId, seq: focusEvent.seq } : undefined);
    setPage(index >= 0 ? Math.floor(index / PAGE_SIZE) : 0);
    setFocusToken({});
  }, [focusEvent?.runId, focusEvent?.seq, focusEvent?.nonce, events]);

  const selectType = (type: string | null) => {
    setSelectedType(type);
    setPage(0);
    setLocated(undefined);
    setIssue(undefined);
  };
  const changePage = (next: number) => {
    setLocated(undefined);
    setFocusToken(undefined);
    setPage(next);
  };
  const locate = () => {
    const seq = /^\d+$/.test(sequence.trim()) ? Number(sequence) : NaN;
    setLocated(undefined);
    if (!Number.isSafeInteger(seq)) { setIssue({ kind: 'required' }); return; }
    let index = -1;
    for (let candidate = 0; candidate < events.length; candidate++) {
      const event = events[candidate];
      if (event.seq !== seq || selectedRun !== null && eventRun(event) !== selectedRun) continue;
      if (index >= 0 && eventRun(event) !== eventRun(events[index])) { setIssue({ kind: 'ambiguous', seq }); return; }
      index = candidate;
    }
    if (index < 0) { setIssue({ kind: 'notFound', seq }); return; }
    setSelectedType(null);
    setPage(Math.floor(index / PAGE_SIZE));
    setLocated({ runId: eventRun(events[index]), seq });
    setIssue(undefined);
    setFocusToken({});
  };
  const issueText = issue ? issue.kind === 'required' ? t('agentReply.diagnosticSequenceRequired') : t(issue.kind === 'ambiguous' ? 'agentReply.diagnosticAmbiguousSequence' : 'agentReply.diagnosticNotFound', { seq: issue.seq }) : undefined;
  if (!events.length && !focusEvent) return null;
  return <details className="agent-reply-disclosure agent-transcript-diagnostics" data-testid="agent-transcript-diagnostics" open={open} onToggle={event => { setOpen(event.currentTarget.open); if (!event.currentTarget.open) { setLocated(undefined); setFocusToken(undefined); } }}>
    <summary><strong>{t('agentReply.diagnostics')}</strong><span>{t('agentReply.diagnosticCount', { count: events.length, types: groups.length })}</span></summary>
    {open ? <div className="agent-reply-detail-body">
      <p className="agent-reply-note">{t('agentReply.diagnosticsHelp')}</p>
      <fieldset className="agent-transcript-diagnostic-types"><legend>{t('agentReply.diagnosticFilterByType')}</legend>
        <Button type="button" size="xs" variant="ghost" aria-pressed={selectedType === null} onClick={() => selectType(null)}>{t('agentReply.diagnosticAllEvents')}<span>{events.length}</span></Button>
        {groups.map(group => <Button type="button" size="xs" variant="ghost" aria-pressed={selectedType === group.key} key={group.key} onClick={() => selectType(group.key)}><code>{group.kind}{group.nativeType ? ` / ${group.nativeType}` : group.kind === 'native/unsupported' ? ` / ${t('agentReply.diagnosticUnknownType')}` : ''}</code><span>{t('agentReply.diagnosticTypeCount', { count: group.count })}</span></Button>)}
      </fieldset>
      <form className="agent-transcript-diagnostic-locate" onSubmit={event => { event.preventDefault(); locate(); }}>
        <Input label={t('agentReply.diagnosticSequence')} value={sequence} inputMode="numeric" error={issueText} onChange={event => { setSequence(event.target.value); setIssue(undefined); }} />
        {runs.length > 1 || selectedRun !== null ? <Select label={t('agentReply.diagnosticRun')} value={selectedRun === null ? 'all' : `run:${selectedRun}`} onChange={value => { setSelectedRun(value === 'all' ? null : value.slice(4)); setIssue(undefined); }} options={[{ value: 'all', label: t('agentReply.diagnosticAllRuns') }, ...[...runs, ...(selectedRun !== null && !runs.includes(selectedRun) ? [selectedRun] : [])].map(run => ({ value: `run:${run}`, label: run || t('agentReply.diagnosticNoRun') }))]} /> : null}
        <Button type="submit" size="xs" variant="secondary">{t('agentReply.diagnosticLocate')}</Button>
      </form>
      <div className="agent-transcript-diagnostic-pagination">
        <span role="status">{t('agentReply.diagnosticRange', { start: filtered.length ? currentPage * PAGE_SIZE + 1 : 0, end: Math.min((currentPage + 1) * PAGE_SIZE, filtered.length), count: filtered.length, total: events.length })}</span>
        <span>{t('agentReply.diagnosticPage', { page: currentPage + 1, pages })}</span>
        <Button type="button" size="xs" variant="ghost" disabled={currentPage === 0} onClick={() => changePage(currentPage - 1)}>{t('agentReply.diagnosticPrevious')}</Button>
        <Button type="button" size="xs" variant="ghost" disabled={currentPage === pages - 1} onClick={() => changePage(currentPage + 1)}>{t('agentReply.diagnosticNext')}</Button>
      </div>
      {pageEvents.length ? <div className="agent-transcript-diagnostic-records">{pageEvents.map((event, index) => <DiagnosticEvent key={JSON.stringify([eventRun(event), event.seq, currentPage * PAGE_SIZE + index])} event={event} highlighted={located !== undefined && eventRun(event) === located.runId && event.seq === located.seq} focusToken={focusToken} />)}</div> : <p className="agent-reply-note">{t('agentReply.diagnosticEmpty')}</p>}
    </div> : null}
  </details>;
}

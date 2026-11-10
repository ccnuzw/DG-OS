import { cleanup, render, screen, fireEvent } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { AgentEvent, AgentSession, Json } from '../src/stores/agentRunnerStore';
import { inspectableAgentEvent, projectCodexTranscript } from '../src/components/development/codexTranscript';
import { CodexTranscriptContent } from '../src/components/development/CodexTranscriptContent';
import { AgentContextDetails } from '../src/components/development/AgentContextDetails';
import { agentContextUsage } from '../src/components/development/agentContextUsage';
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
afterEach(cleanup);
const event = (seq: number, kind: string, data: Json, run = 'run-a', text: string | null = null): AgentEvent => ({ seq, kind, data, run, text });
const delta = (seq: number, itemId: string, summaryIndex: number, text: string, run = 'run-a') => event(seq, 'item/reasoning/summaryTextDelta', { itemId, summaryIndex, delta: text }, run, text);

it('streams summaries independently by run, item and index and replaces only the completed matching item', () => {
  const events = [
    event(1, 'item/reasoning/summaryPartAdded', { itemId: 'one', summaryIndex: 0 }),
    delta(2, 'one', 0, 'Draft '), delta(3, 'one', 1, 'Second part'),
    delta(4, 'one', 0, 'Other turn', 'run-b'), delta(5, 'two', 0, 'Other item'),
    delta(6, 'one', 0, 'summary'),
  ];
  const streaming = projectCodexTranscript(events);
  expect(streaming.get(events[0])).toMatchObject({ kind: 'summary', parts: [{ index: 0, text: 'Draft summary' }], complete: false, seq: 6 });
  expect(streaming.get(events[2])).toMatchObject({ parts: [{ index: 1, text: 'Second part' }] });
  const completed = event(7, 'reasoning', { id: 'one', type: 'reasoning', summary: ['Revised native summary'], content: ['PRIVATE RAW REASONING'] });
  events.push(completed);
  const projection = projectCodexTranscript(events);
  expect(projection.get(events[0])).toBeNull();
  expect(projection.get(events[2])).toBeNull();
  expect(projection.get(events[3])).toMatchObject({ parts: [{ text: 'Other turn' }] });
  expect(projection.get(events[4])).toMatchObject({ parts: [{ text: 'Other item' }] });
  expect(projection.get(completed)).toMatchObject({ parts: [{ index: 0, text: 'Revised native summary' }], complete: true });
  expect(JSON.stringify(projection.get(completed))).not.toContain('PRIVATE');
  expect(completed.data).toHaveProperty('content');
});

it('keeps malformed summary notifications inspectable without destroying a valid stream', () => {
  const first = delta(1, 'one', 0, 'Still streaming');
  const badIndex = delta(2, 'one', Number.MAX_SAFE_INTEGER + 1, 'bad');
  const badSnapshot = event(3, 'reasoning', { id: 'one', summary: ['valid', 42] });
  const projection = projectCodexTranscript([first, badIndex, badSnapshot]);
  expect(projection.get(first)).toMatchObject({ kind: 'summary' });
  expect(projection.get(badIndex)).toEqual({ kind: 'malformed' });
  expect(projection.get(badSnapshot)).toEqual({ kind: 'malformed' });
  render(<CodexTranscriptContent event={badSnapshot} entry={projection.get(badSnapshot)!} inspector={value => <pre>{JSON.stringify(inspectableAgentEvent(value))}</pre>} />);
  expect(screen.getByText('codexTranscript.malformed')).toBeTruthy();
  expect(screen.getByText(/42/)).toBeTruthy();
});

it('does not join unscoped events, and part-added never resets existing streamed text', () => {
  const first = delta(1, 'one', 0, 'A');
  const part = event(2, 'item/reasoning/summaryPartAdded', { itemId: 'one', summaryIndex: 0 });
  const last = delta(3, 'one', 0, 'B');
  expect(projectCodexTranscript([first, part, last]).get(first)).toMatchObject({ parts: [{ text: 'AB' }] });
  const unscoped = [{ ...first, run: null }, { ...last, run: null }];
  const projection = projectCodexTranscript(unscoped);
  expect(projection.get(unscoped[0])).toMatchObject({ parts: [{ text: 'A' }] });
  expect(projection.get(unscoped[1])).toMatchObject({ parts: [{ text: 'B' }] });
});

it('renders reasoning summaries collapsed and never renders raw reasoning in the native inspector', async () => {
  const completed = event(1, 'reasoning', { type: 'reasoning', id: 'r', summary: ['Public summary'], content: ['PRIVATE'], encryptedContent: 'SECRET' }, 'run-a', 'RAW TEXT');
  const entry = projectCodexTranscript([completed]).get(completed)!;
  const view = render(<CodexTranscriptContent event={completed} entry={entry} inspector={() => null} />);
  expect(screen.getByTestId('codex-reasoning-summary').hasAttribute('open')).toBe(false);
  expect(screen.queryByText('Public summary')).toBeNull();
  fireEvent.click(screen.getByText('codexTranscript.summary'));
  expect(await screen.findByText('Public summary')).toBeTruthy();
  expect(view.container.textContent).not.toContain('PRIVATE');
  const inspected = inspectableAgentEvent(completed);
  expect(inspected.text).toBeNull();
  expect(JSON.stringify(inspected)).not.toMatch(/PRIVATE|SECRET|RAW TEXT/);
  const nested = event(2, 'rawResponseItem/completed', { item: completed.data }, 'run-a', 'RAW TEXT');
  expect(JSON.stringify(inspectableAgentEvent(nested))).not.toMatch(/PRIVATE|SECRET|RAW TEXT/);
  expect(inspectableAgentEvent(delta(3, 'one', 0, 'Public delta')).text).toBe('Public delta');
});

it('retains the latest plan and diff per run with revision history, explanation and native step status', () => {
  const first = event(1, 'turn/plan/updated', { explanation: 'Original reason', plan: [{ step: 'Inspect', status: 'pending' }] });
  const diff = event(2, 'turn/diff/updated', { diff: '+original' });
  const other = event(3, 'turn/plan/updated', { plan: [{ step: 'Other run', status: 'pending' }] }, 'run-b');
  const latest = event(4, 'turn/plan/updated', { explanation: 'Changed reason', plan: [{ step: 'Inspect', status: 'completed' }, { step: 'Review', status: 'inProgress' }, { step: 'Native status', status: 'customState' }] });
  const newestDiff = event(5, 'turn/diff/updated', { diff: '+replacement' });
  const projection = projectCodexTranscript([first, diff, other, latest, newestDiff]);
  expect(projection.get(first)).toBeNull(); expect(projection.get(diff)).toBeNull();
  expect(projection.get(other)).toMatchObject({ revisions: [other] });
  expect(projection.get(latest)).toMatchObject({ revisions: [first, latest] });
  expect(projection.get(newestDiff)).toMatchObject({ revisions: [diff, newestDiff] });
  const view = render(<CodexTranscriptContent event={latest} entry={projection.get(latest)!} inspector={() => null} />);
  expect(view.container.textContent).toContain('Changed reason');
  expect(view.container.textContent).toContain('Original reason');
  expect(view.container.textContent).toContain('execution.completed');
  expect(view.container.textContent).toContain('execution.in_progress');
  expect(view.container.textContent).toContain('customState');
  expect(view.container.textContent).toContain('codexTranscript.proposalBoundary');
  fireEvent.click(screen.getByText('codexTranscript.previousRevisions'));
  expect(screen.getByText('codexTranscript.previousRevisions').closest('details')?.open).toBe(true);
});

it('separates latest and cumulative counts, rejects overflow, and identifies active-run snapshots', () => {
  const value: AgentSession = { id: 's', generation: 'g', engine: 'codex', native_id: 'n', workspace: '/', status: 'running', active_run: 'next-run', pending: [], error: null, events: [event(12, 'thread/tokenUsage/updated', { tokenUsage: { last: { inputTokens: 100, outputTokens: 20, cachedInputTokens: 40, reasoningOutputTokens: 5, totalTokens: 120 }, total: { inputTokens: 900, outputTokens: 200, cachedInputTokens: 300, reasoningOutputTokens: 90, totalTokens: 1100 }, modelContextWindow: 1000 } })] };
  const usage = agentContextUsage(value);
  expect(usage).toMatchObject({ used: 120, limit: 1000, seq: 12, previous: true, latest: { input: 100, output: 20, cached: 40, reasoning: 5, total: 120 }, cumulative: { input: 900, output: 200, cached: 300, reasoning: 90, total: 1100 } });
  expect(usage.source).toContain('tokenUsage.last'); expect(usage.source).toContain('tokenUsage.total');
  render(<AgentContextDetails usage={usage} />);
  expect(screen.getByRole('region', { name: 'codexTranscript.latestUsage' }).textContent).toContain('100');
  expect(screen.getByRole('region', { name: 'codexTranscript.cumulativeUsage' }).textContent).toContain('900');
  expect(screen.getByRole('status').textContent).toBe('agentControls.previousUsage');
  expect(screen.getByText('#12')).toBeTruthy();
  value.events.push(event(13, 'thread/tokenUsage/updated', { tokenUsage: { last: { inputTokens: Number.MAX_SAFE_INTEGER, outputTokens: 1, cachedInputTokens: -1, reasoningOutputTokens: 0.5 }, total: { inputTokens: Number.MAX_SAFE_INTEGER + 1 }, modelContextWindow: Number.MAX_SAFE_INTEGER + 1 } }));
  expect(agentContextUsage(value)).toMatchObject({ used: null, limit: null, kind: 'unavailable', seq: 13, latest: { cached: null, reasoning: null }, cumulative: { input: null } });
  value.events.push(event(14, 'thread/tokenUsage/updated', { tokenUsage: { total: { totalTokens: 999999 } } }));
  expect(agentContextUsage(value)).toMatchObject({ used: null, limit: null, seq: 14, latest: { input: null, output: null } });
});

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { AgentReplyTimeline } from '../src/components/development/AgentReplyTimeline';
import { EventContent } from '../src/components/development/AgentReplyContent';
import { findAgentTranscriptRow, projectAgentTranscript } from '../src/components/development/agentTranscript';
import type { AgentEvent, Json } from '../src/stores/agentRunnerStore';
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string, args?: Record<string, unknown>) => args ? `${key} ${Object.values(args).join(' ')}` : key }), t: (key: string, args?: Record<string, unknown>) => args ? `${key} ${Object.values(args).join(' ')}` : key }));
vi.mock(import('../src/components/development/AgentReplyContent'), async importOriginal => {
  const original = await importOriginal();
  return { ...original, EventContent: vi.fn(original.EventContent) };
});
afterEach(() => { cleanup(); vi.mocked(EventContent).mockClear(); vi.restoreAllMocks(); });
const read = (seq: number, run: string, failed = false): AgentEvent => ({seq, run, kind:'commandExecution', text:null, data:{id:`read-${seq}`,type:'commandExecution',command:`cat file-${seq}`,commandActions:[{type:'read'}],status:failed?'failed':'completed',exitCode:failed?1:0,aggregatedOutput:failed?'permission denied':`content-${seq}`}});
it('groups adjacent successful reads without hiding a failure or crossing a user turn', () => {
  const projection = projectAgentTranscript([read(1,'a'),read(2,'a'),read(3,'a',true),{seq:4,run:'a',kind:'userMessage',text:'Next question',data:null},read(5,'a'),read(6,'b')]);
  const view = render(<AgentReplyTimeline {...projection} runStates={new Map()} engine="codex"/>);
  const group = view.container.querySelector('.agent-reply-read-group') as HTMLDetailsElement;
  expect(group.open).toBe(false);
  expect(group.querySelectorAll('[data-event-kind="commandExecution"]')).toHaveLength(2);
  expect(Array.from(view.container.querySelectorAll('.agent-reply-read-group'), node => node.querySelectorAll('[data-reply-key]').length)).toEqual([2]);
  expect(view.container.querySelectorAll('.agent-reply-single-read')).toHaveLength(2);
  expect(screen.getByText('agentExecution.failed').closest('.agent-reply-read-group')).toBeNull();
  expect(screen.getByText('Next question')).toBeTruthy();
  fireEvent.click(group.querySelector('summary')!);
  expect(group.open).toBe(true);
});
it('preserves an expanded single read and its details when more reads and an answer arrive', async () => {
  const events = [read(1,'a')];
  const initial = projectAgentTranscript(events);
  const view = render(<AgentReplyTimeline {...initial} runStates={new Map()} engine="codex"/>);
  expect(view.container.querySelector('.agent-reply-read-group')).toBeNull();
  const container = view.container.querySelector('.agent-reply-single-read') as HTMLDetailsElement;
  expect(container.open).toBe(true);
  expect(container.querySelector('summary')?.hidden).toBe(true);
  expect(container.querySelector('summary')?.tabIndex).toBe(-1);
  const operation = container.querySelector('.agent-reply-operation') as HTMLDetailsElement;
  fireEvent.click(operation.querySelector('summary')!);
  await waitFor(() => expect(within(operation).getByText('content-1')).toBeTruthy());
  const next = projectAgentTranscript([...events,read(2,'a'),{seq:3,run:'a',kind:'agentMessage',text:'Completed answer',data:{id:'answer',phase:'final_answer'}}]);
  view.rerender(<AgentReplyTimeline {...next} runStates={new Map()} engine="codex"/>);
  const group = view.container.querySelector('.agent-reply-read-group') as HTMLDetailsElement;
  expect(group).toBe(container);
  expect(group.open).toBe(true);
  expect(group.querySelector('.agent-reply-operation')).toBe(operation);
  expect(operation.open).toBe(true);
  expect(group.querySelectorAll('[data-reply-key]')).toHaveLength(2);
  expect(screen.getByText('Completed answer')).toBeTruthy();
});

it('RA04 jumps to the explicit final answer and never uses commentary, a child message or the user as an answer target', () => {
  const scroll = vi.fn();
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { configurable: true, value: scroll });
  const events: AgentEvent[] = [
    { seq: 1, run: 'a', kind: 'userMessage', text: 'Question', data: null },
    { seq: 2, run: 'a', kind: 'agentMessage', text: 'Checking first', data: { id: 'commentary', phase: 'commentary' } },
    read(3, 'a'), read(4, 'a'), read(5, 'a'),
    { seq: 6, run: 'a', kind: 'fileChange', text: null, data: { id: 'change', type: 'fileChange', changes: [{ path: 'one.ts', kind: { type: 'update' }, diff: '+new' }] } },
    { seq: 7, run: 'a', kind: 'agentMessage', text: 'Child answer', data: { id: 'child', parent_tool_use_id: 'spawn', phase: 'final_answer' } },
    { seq: 8, run: 'a', kind: 'agentMessage', text: 'Final answer', data: { id: 'answer', phase: 'final_answer' } },
    { seq: 9, run: 'a', kind: 'agentMessage', text: 'Phase unavailable', data: { id: 'later' } },
  ];
  const projection = projectAgentTranscript(events);
  const view = render(<AgentReplyTimeline {...projection} runStates={new Map()} engine="codex" />);
  fireEvent.click(screen.getByRole('button', { name: 'agentExecution.jumpAnswer' }));
  const final = screen.getByTestId('agent-event-8').closest('[data-reply-key]');
  expect(document.activeElement).toBe(final);
  expect(final?.getAttribute('data-reply-key')).toBe(projection.rows.find(row => row.event.seq === 8)?.key);
  expect(screen.getByTestId('agent-event-1').closest('[data-reply-section]')).toBeNull();
  expect(screen.getByTestId('agent-event-2').closest('[data-reply-section]')?.getAttribute('data-reply-section')).toBe('process');
  expect(screen.getByTestId('agent-event-7').closest('[data-reply-section]')?.getAttribute('data-reply-section')).toBe('process');
  fireEvent.click(screen.getByRole('button', { name: 'agentExecution.jumpProcess' }));
  expect(document.activeElement).toBe(screen.getByTestId('agent-event-2').closest('[data-reply-key]'));
  expect(scroll).toHaveBeenCalledTimes(2);
  expect(view.container.querySelector('.agent-reply-read-group')).toBeTruthy();
});

it('RA04 reveals a collapsed read group when activity navigation targets its first operation', () => {
  const scroll = vi.fn();
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { configurable: true, value: scroll });
  const events: AgentEvent[] = [
    { seq: 1, run: 'a', kind: 'userMessage', text: 'Question', data: null },
    ...Array.from({ length: 6 }, (_, index) => read(index + 2, 'a')),
    { seq: 8, run: 'a', kind: 'agentMessage', text: 'Answer', data: { id: 'answer', phase: 'final_answer' } },
  ];
  const projection = projectAgentTranscript(events);
  const view = render(<AgentReplyTimeline {...projection} runStates={new Map()} engine="codex" />);
  const group = view.container.querySelector('.agent-reply-read-group') as HTMLDetailsElement;
  expect(group.open).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'agentExecution.jumpProcess' }));
  expect(group.open).toBe(true);
  expect(document.activeElement).toBe(screen.getByTestId('agent-event-2').closest('[data-reply-key]'));
  expect(scroll).toHaveBeenCalledOnce();
});

it('does not count empty block starts and metadata towards the reply navigation threshold', () => {
  const events: AgentEvent[] = [
    ...Array.from({ length: 10 }, (_, index): AgentEvent => ({ seq: index + 1, run: 'a', kind: 'content_block_start', text: null, data: { native_message_id: 'm', block_index: index, content_block: { type: 'text', text: '' } } })),
    { seq: 11, run: 'a', kind: 'thread/tokenUsage/updated', text: null, data: { tokens: 20 } },
    read(12, 'a'),
    { seq: 13, run: 'a', kind: 'agentMessage', text: 'Answer', data: { id: 'answer', phase: 'final_answer' } },
  ];
  const projection = projectAgentTranscript(events);
  const view = render(<AgentReplyTimeline {...projection} runStates={new Map()} engine="codex" />);
  expect(view.container.querySelectorAll('[data-reply-key]')).toHaveLength(2);
  expect(screen.queryByRole('navigation', { name: 'agentExecution.replyNavigation' })).toBeNull();
});

it('forwards reply actions and native identity uncertainty without invoking an action during render', () => {
  const projection = projectAgentTranscript([{ seq: 1, run: 'a', kind: 'message', text: null, data: { type: 'assistant', content: [{ type: 'text', text: 'Unlinked snapshot' }] } }]);
  const onQuoteReply = vi.fn(), onOpenAgentActivity = vi.fn(), onPreviewResource = vi.fn();
  render(<AgentReplyTimeline {...projection} runStates={new Map()} onQuoteReply={onQuoteReply} onOpenAgentActivity={onOpenAgentActivity} onPreviewResource={onPreviewResource} />);
  expect(vi.mocked(EventContent).mock.calls.at(-1)?.[0]).toMatchObject({ identityUncertain: true, onQuoteReply, onOpenAgentActivity, onPreviewResource });
  expect(onQuoteReply).not.toHaveBeenCalled();
  expect(onOpenAgentActivity).not.toHaveBeenCalled();
  expect(onPreviewResource).not.toHaveBeenCalled();
});

it('keeps failed native read results outside successful read groups', () => {
  const events = ['one', 'two', 'three'].flatMap<AgentEvent>((id, index) => [
    { seq: index * 2 + 1, run: 'a', kind: 'message', text: null, data: { type: 'assistant', content: [{ type: 'tool_use', id, name: 'Read', input: { file_path: `${id}.ts` } }] } },
    { seq: index * 2 + 2, run: 'a', kind: 'message', text: null, data: { type: 'user', content: [{ type: 'tool_result', tool_use_id: id, isError: index === 2, content: index === 2 ? 'Permission denied' : 'Read contents' }] } },
  ]);
  const view = render(<AgentReplyTimeline {...projectAgentTranscript(events)} runStates={new Map()} engine="claude" />);
  expect(view.container.querySelector('.agent-reply-read-group')?.querySelectorAll('[data-reply-key]')).toHaveLength(2);
  expect(screen.getByText('agentExecution.failed').closest('.agent-reply-read-group')).toBeNull();
  expect(screen.getByRole('alert').textContent).toContain('Permission denied');
});

it('excludes buffered prose and child changes from answer and parent changes navigation', () => {
  const events: AgentEvent[] = [
    ...Array.from({ length: 6 }, (_, index) => read(index + 1, 'a')),
    { seq: 7, run: 'a', kind: 'fileChange', text: null, data: { type: 'fileChange', parent_tool_use_id: 'child', changes: [{ path: 'child.ts' }] } },
    { seq: 8, run: 'a', kind: 'agentMessage', text: 'Buffered answer', data: { id: 'answer', phase: 'final_answer' } },
  ];
  const projection = projectAgentTranscript(events);
  const view = render(<AgentReplyTimeline {...projection} runStates={new Map()} engine="codex" bufferingRun="a" />);
  expect(screen.queryByText('Buffered answer')).toBeNull();
  expect(view.container.querySelector('[data-reply-section="answer"]')).toBeNull();
  expect(view.container.querySelector('[data-reply-section="changes"]')).toBeNull();
  expect(screen.queryByRole('navigation', { name: 'agentExecution.replyNavigation' })).toBeNull();
  view.rerender(<AgentReplyTimeline {...projection} runStates={new Map()} engine="codex" />);
  expect(screen.getByRole('button', { name: 'agentExecution.jumpAnswer' })).toBeTruthy();
  expect(screen.getByTestId('agent-event-7').closest('[data-reply-section]')?.getAttribute('data-reply-section')).toBe('process');
});

it('jumps to the newest explicit final answer when a turn contains multiple final snapshots', () => {
  const scroll = vi.fn();
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { configurable: true, value: scroll });
  const events: AgentEvent[] = [
    ...Array.from({ length: 6 }, (_, index) => read(index + 1, 'a')),
    { seq: 7, run: 'a', kind: 'agentMessage', text: 'Earlier final answer', data: { id: 'earlier', phase: 'final_answer' } },
    { seq: 8, run: 'a', kind: 'agentMessage', text: 'Newest final answer', data: { id: 'newest', phase: 'final_answer' } },
  ];
  render(<AgentReplyTimeline {...projectAgentTranscript(events)} runStates={new Map()} engine="codex" />);
  fireEvent.click(screen.getByRole('button', { name: 'agentExecution.jumpAnswer' }));
  expect(document.activeElement).toBe(screen.getByTestId('agent-event-8').closest('[data-reply-key]'));
  expect(scroll).toHaveBeenCalledOnce();
});

it('recognizes a visible Claude terminal answer without inventing an explicit final phase', () => {
  const events: AgentEvent[] = [
    ...Array.from({ length: 7 }, (_, index) => read(index + 1, 'a')),
    { seq: 8, run: 'a', kind: 'result', text: null, data: { result: 'Only terminal answer', run_status: 'completed' } },
  ];
  const view = render(<AgentReplyTimeline {...projectAgentTranscript(events)} runStates={new Map()} engine="claude" />);
  expect(screen.getByText('Only terminal answer').closest('[data-reply-section]')?.getAttribute('data-reply-section')).toBe('answer');
  expect(screen.getByRole('button', { name: 'agentExecution.jumpAnswer' })).toBeTruthy();
  expect(view.container.querySelector('[data-reply-final]')).toBeNull();
});

it('keeps successful read groups in place around prose, failed operations and uncertain commands', () => {
  const command = (seq: number, data: Record<string, Json>): AgentEvent => ({ ...read(seq, 'a'), data: { id: `read-${seq}`, type: 'commandExecution', command: `command-${seq}`, status: 'completed', exitCode: 0, ...data } });
  const events: AgentEvent[] = [
    read(1, 'a'), read(2, 'a'),
    { seq: 3, run: 'a', kind: 'agentMessage', text: 'Intermediate answer', data: { id: 'intermediate' } },
    read(4, 'a'), read(5, 'a', true),
    command(6, { commandActions: [{ type: 'read' }, { type: 'unknown' }] }),
    command(7, { commandActions: [] }),
    command(8, { commandActions: [{ type: 'search' }], status: 'waiting' }),
    command(9, { commandActions: [{ type: 'read' }], status: 'declined' }),
    command(10, { commandActions: [{ type: 'listFiles' }], status: 'cancelled' }),
    command(11, { commandActions: [{ type: 'read' }], status: 'inProgress' }),
    read(12, 'a'),
    { seq: 13, run: 'a', kind: 'agentMessage', text: 'Final answer', data: { id: 'final', phase: 'final_answer' } },
  ];
  const projection = projectAgentTranscript(events);
  const view = render(<AgentReplyTimeline {...projection} runStates={new Map()} engine="codex" />);
  const groups = view.container.querySelectorAll('.agent-reply-read-group');
  expect(Array.from(groups, group => Array.from(group.querySelectorAll('[data-event-seq]'), node => Number(node.getAttribute('data-event-seq'))))).toEqual([[1, 2]]);
  expect(view.container.querySelectorAll('.agent-reply-single-read')).toHaveLength(2);
  expect(Array.from(view.container.querySelectorAll('[data-reply-key]'), node => node.getAttribute('data-reply-key'))).toEqual(projection.rows.map(row => row.key));
  for (const seq of [3, 5, 6, 7, 8, 9, 10, 11, 13]) expect(screen.getByTestId(`agent-event-${seq}`).closest('.agent-reply-read-group')).toBeNull();
  expect(screen.getByRole('alert').textContent).toContain('permission denied');
});

it('keeps a standalone native read searchable and retains source evidence inside its operation disclosure', async () => {
  const events: AgentEvent[] = [
    { seq: 1, run: 'a', kind: 'message', text: null, data: { type: 'assistant', content: [{ type: 'tool_use', id: 'native-read', name: 'Read', input: { file_path: 'source.ts' } }] } },
    { seq: 2, run: 'a', kind: 'message', text: null, data: { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'native-read', content: 'Native source contents' }] } },
  ];
  const projection = projectAgentTranscript(events);
  const view = render(<AgentReplyTimeline {...projection} runStates={new Map()} engine="claude" />);
  const row = findAgentTranscriptRow(projection.rows, { run: 'a', seq: 2 })!;
  const anchor = Array.from(view.container.querySelectorAll<HTMLElement>('[data-reply-key]')).find(node => node.dataset.replyKey === row.key)!;
  const event = anchor.querySelector<HTMLElement>('[data-event-seq]')!;
  const group = anchor.closest('.agent-reply-single-read') as HTMLDetailsElement;
  expect(anchor.closest('.agent-reply-read-group')).toBeNull();
  expect(group.open).toBe(true);
  expect(row.sources).toEqual(events);
  expect(vi.mocked(EventContent).mock.calls.at(-1)?.[0]).toMatchObject({ sources: events, engine: 'claude' });
  // History search reveals ancestor disclosures before focusing the retained event.
  for (let ancestor = event.parentElement; ancestor; ancestor = ancestor.parentElement) if (ancestor instanceof HTMLDetailsElement) ancestor.open = true;
  event.tabIndex = -1;
  event.focus();
  expect(document.activeElement).toBe(event);
  expect(group.open).toBe(true);
  const operation = anchor.querySelector('.agent-reply-operation') as HTMLDetailsElement;
  fireEvent.click(operation.querySelector('summary')!);
  await waitFor(() => expect(operation.querySelector('.chat-code code')?.textContent).toBe('Native source contents'));
  const native = operation.querySelector('.agent-reply-native') as HTMLDetailsElement;
  fireEvent.click(native.querySelector('summary')!);
  await waitFor(() => expect(native.querySelectorAll('pre')).toHaveLength(2));
  expect(native.textContent).toContain('native-read');
  expect(native.textContent).toContain('Native source contents');
  expect(document.activeElement).toBe(event);
});

it('does not collapse native read evidence that also carries assistant prose or a child tool scope', () => {
  const events: AgentEvent[] = [
    { seq: 1, run: 'a', kind: 'message', text: 'Visible explanation', data: { type: 'assistant', content: [{ type: 'tool_use', id: 'read-with-prose', name: 'Read', input: { file_path: 'one.ts' } }] } },
    { seq: 2, run: 'a', kind: 'message', text: null, data: { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'read-with-prose', content: 'Read output' }] } },
    { seq: 3, run: 'a', kind: 'message', text: null, data: { type: 'assistant', parent_tool_use_id: 'child', content: [{ type: 'tool_use', id: 'child-read', name: 'Read', input: { file_path: 'two.ts' } }] } },
    { seq: 4, run: 'a', kind: 'message', text: null, data: { type: 'user', parent_tool_use_id: 'child', content: [{ type: 'tool_result', tool_use_id: 'child-read', content: 'Child output' }] } },
  ];
  const view = render(<AgentReplyTimeline {...projectAgentTranscript(events)} runStates={new Map()} engine="claude" />);
  expect(view.container.querySelector('.agent-reply-read-group')).toBeNull();
  expect(screen.getByText('Visible explanation')).toBeTruthy();
  expect(view.container.querySelector('.agent-reply-subtask-message')).toBeTruthy();
});

it.each([
  ['completed', 'replyCompleted'],
  ['failed', 'failed'],
  ['cancelled', 'stopped'],
  [null, 'unknown'],
] as const)('retains the %s terminal state while exposing compact styling only for a completed reply', (runStatus, state) => {
  const terminal: AgentEvent = { seq: 2, run: 'a', kind: 'result', text: null, data: runStatus ? { run_status: runStatus } : {} };
  const view = render(<AgentReplyTimeline {...projectAgentTranscript([read(1, 'a'), terminal])} runStates={new Map([['a', 'stopped']])} engine="claude" />);
  const event = screen.getByTestId('agent-event-2');
  expect(event.closest('[data-reply-key]')?.getAttribute('data-reply-terminal')).toBe(state);
  expect(event.querySelector('[data-operation-state]')?.getAttribute('data-operation-state')).toBe(state);
  expect(event.closest('.agent-reply-read-group')).toBeNull();
  expect(view.container.querySelector('.agent-reply-read-group')).toBeNull();
  expect(view.container.querySelector('.agent-reply-single-read')).toBeTruthy();
});

it('summarizes native Codex actions and never classifies a shell command by its text', () => {
  const events: AgentEvent[] = [
    { ...read(1, 'a'), data: { id: 'one', type: 'commandExecution', status: 'completed', exitCode: 0, command: 'opaque command', commandActions: [{ type: 'read' }, { type: 'read' }, { type: 'search' }] } },
    { ...read(2, 'a'), data: { id: 'two', type: 'commandExecution', status: 'completed', exitCode: 0, command: 'another opaque command', commandActions: [{ type: 'listFiles' }] } },
    { ...read(3, 'a'), data: { id: 'unknown', type: 'commandExecution', status: 'completed', exitCode: 0, command: 'cat file.ts && rg needle . && ls', commandActions: [{ type: 'unknown' }] } },
  ];
  const view = render(<AgentReplyTimeline {...projectAgentTranscript(events)} runStates={new Map()} engine="codex" />);
  const group = view.container.querySelector('.agent-reply-read-group')!;
  const summary = group.querySelector('summary')!;
  expect(summary.textContent).toContain('agentReply.readSummaryFiles 2');
  expect(summary.textContent).toContain('agentReply.readSummarySearches 1');
  expect(summary.textContent).toContain('agentReply.readSummaryDirectories 1');
  expect(summary.textContent).toContain('agentExecution.operationCount 2');
  expect(screen.getByTestId('agent-event-3').closest('.agent-reply-read-group')).toBeNull();
});

it('summarizes Claude Read and native pattern searches without guessing directory operations', () => {
  const events = ['Read', 'Grep', 'Glob'].flatMap<AgentEvent>((name, index) => [
    { seq: index * 2 + 1, run: 'a', kind: 'message', text: null, data: { type: 'assistant', content: [{ type: 'tool_use', id: `tool-${index}`, name, input: { file_path: 'source.ts', path: '.', pattern: 'needle' } }] } },
    { seq: index * 2 + 2, run: 'a', kind: 'message', text: null, data: { type: 'user', content: [{ type: 'tool_result', tool_use_id: `tool-${index}`, content: 'Native result' }] } },
  ]);
  const view = render(<AgentReplyTimeline {...projectAgentTranscript(events)} runStates={new Map()} engine="claude" />);
  const summary = view.container.querySelector('.agent-reply-read-group > summary')!;
  expect(summary.textContent).toContain('agentReply.readSummaryFiles 1');
  expect(summary.textContent).toContain('agentReply.readSummarySearches 2');
  expect(summary.textContent).not.toContain('agentReply.readSummaryDirectories');
});

it('starts a newly formed group collapsed when the single operation was never opened', () => {
  const first = projectAgentTranscript([read(1, 'a')]);
  const view = render(<AgentReplyTimeline {...first} runStates={new Map()} engine="codex" />);
  const container = view.container.querySelector('.agent-reply-single-read') as HTMLDetailsElement;
  const operation = container.querySelector('.agent-reply-operation');
  view.rerender(<AgentReplyTimeline {...projectAgentTranscript([read(1, 'a'), read(2, 'a')])} runStates={new Map()} engine="codex" />);
  expect(view.container.querySelector('.agent-reply-read-group')).toBe(container);
  expect(container.open).toBe(false);
  expect(container.querySelector('.agent-reply-operation')).toBe(operation);
});

it('retains a grouped read anchor for full history lookup and reveals all ancestor disclosures', () => {
  const events = [read(1, 'a'), read(2, 'a')];
  const projection = projectAgentTranscript(events);
  const view = render(<AgentReplyTimeline {...projection} runStates={new Map()} engine="codex" />);
  const row = findAgentTranscriptRow(projection.rows, { run: 'a', seq: 2 })!;
  const anchor = Array.from(view.container.querySelectorAll<HTMLElement>('[data-reply-key]')).find(node => node.dataset.replyKey === row.key)!;
  const group = anchor.closest('.agent-reply-read-group') as HTMLDetailsElement;
  expect(group.open).toBe(false);
  for (let ancestor = anchor.parentElement; ancestor; ancestor = ancestor.parentElement) if (ancestor instanceof HTMLDetailsElement) ancestor.open = true;
  anchor.tabIndex = -1;
  anchor.focus();
  expect(group.open).toBe(true);
  expect(document.activeElement).toBe(anchor);
});

it('keeps a focused closed read visible when its singleton becomes a group', () => {
  const view = render(<AgentReplyTimeline {...projectAgentTranscript([read(1, 'a')])} runStates={new Map()} engine="codex" />);
  const container = view.container.querySelector('.agent-reply-single-read') as HTMLDetailsElement;
  const summary = container.querySelector<HTMLElement>('.agent-reply-operation > summary')!;
  summary.focus();
  view.rerender(<AgentReplyTimeline {...projectAgentTranscript([read(1, 'a'), read(2, 'a')])} runStates={new Map()} engine="codex" />);
  expect(container.open).toBe(true);
  expect(document.activeElement).toBe(summary);
  expect(container.querySelector('.agent-reply-operation')?.hasAttribute('open')).toBe(false);
});

it('moves focus from a disappearing group summary into the remaining singleton operation', () => {
  const events = [read(1, 'a'), read(2, 'a')];
  const view = render(<AgentReplyTimeline {...projectAgentTranscript(events)} runStates={new Map()} engine="codex" />);
  const container = view.container.querySelector('.agent-reply-read-group') as HTMLDetailsElement;
  (container.querySelector('summary') as HTMLElement).focus();
  view.rerender(<AgentReplyTimeline {...projectAgentTranscript(events.slice(0, 1))} runStates={new Map()} engine="codex" />);
  expect(container.open).toBe(true);
  expect(document.activeElement).toBe(container.querySelector('.agent-reply-operation > summary'));
});

it('does not group restored reads across native sessions or anonymous evidence', () => {
  const events = [
    { ...read(1, 'a'), run: null, data: { ...read(1, 'a').data as Record<string, Json>, session_id: 'one' } },
    { ...read(2, 'a'), run: null, data: { ...read(2, 'a').data as Record<string, Json>, session_id: 'one' } },
    { ...read(3, 'a'), run: null, data: { ...read(3, 'a').data as Record<string, Json>, session_id: 'two' } },
    { ...read(4, 'a'), run: null },
    { ...read(5, 'a'), run: null },
  ];
  const view = render(<AgentReplyTimeline {...projectAgentTranscript(events)} runStates={new Map()} engine="codex" />);
  expect(view.container.querySelectorAll('.agent-reply-turn')).toHaveLength(4);
  expect(view.container.querySelectorAll('.agent-reply-read-group')).toHaveLength(1);
  expect(view.container.querySelector('.agent-reply-read-group')?.querySelectorAll('[data-reply-key]')).toHaveLength(2);
  expect(view.container.querySelectorAll('.agent-reply-single-read')).toHaveLength(3);
});

it.each(['pending', 'waiting', 'declined', 'unknown'])('keeps a Claude %s result out of successful read groups', status => {
  const events = [1, 2].flatMap<AgentEvent>((seq, index) => [
    { seq: seq * 2 - 1, run: 'a', kind: 'message', text: null, data: { type: 'assistant', content: [{ type: 'tool_use', id: `read-${seq}`, name: 'Read', input: {} }] } },
    { seq: seq * 2, run: 'a', kind: 'message', text: null, data: { type: 'user', content: [{ type: 'tool_result', tool_use_id: `read-${seq}`, ...(index ? { status } : {}), content: 'Retained output' }] } },
  ]);
  const view = render(<AgentReplyTimeline {...projectAgentTranscript(events)} runStates={new Map([['a', 'processing']])} engine="claude" />);
  expect(view.container.querySelector('.agent-reply-read-group')).toBeNull();
  expect(view.container.querySelectorAll('.agent-reply-single-read')).toHaveLength(1);
  const expected = ['pending', 'waiting'].includes(status) ? 'waiting' : status;
  expect(screen.getByText(`agentExecution.${expected}`).closest('.agent-reply-single-read')).toBeNull();
});

it('keeps a failed assistant final phase in process navigation even while prose is buffered', () => {
  const failed: AgentEvent = { seq: 1, run: 'a', kind: 'agentMessage', text: null, data: { id: 'failed-answer', phase: 'final_answer', errors: ['Native answer failed'] } };
  render(<AgentReplyTimeline {...projectAgentTranscript([failed])} runStates={new Map()} engine="codex" bufferingRun="a" />);
  const row = screen.getByTestId('agent-event-1').closest('[data-reply-key]');
  expect(row?.getAttribute('data-reply-section')).toBe('process');
  expect(row?.hasAttribute('data-reply-final')).toBe(false);
  expect(screen.getByRole('alert').textContent).toContain('Native answer failed');
});

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { AgentReplyTimeline } from '../src/components/development/AgentReplyTimeline';
import { EventContent } from '../src/components/development/AgentReplyContent';
import { projectAgentTranscript } from '../src/components/development/agentTranscript';
import type { AgentEvent } from '../src/stores/agentRunnerStore';
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));
vi.mock(import('../src/components/development/AgentReplyContent'), async importOriginal => {
  const original = await importOriginal();
  return { ...original, EventContent: vi.fn(original.EventContent) };
});
afterEach(() => { cleanup(); vi.mocked(EventContent).mockClear(); vi.restoreAllMocks(); });
const read = (seq: number, run: string, failed = false): AgentEvent => ({seq, run, kind:'commandExecution', text:null, data:{id:`read-${seq}`,type:'commandExecution',command:`cat file-${seq}`,commandActions:[{type:'read'}],status:failed?'failed':'completed',exitCode:failed?1:0,aggregatedOutput:failed?'permission denied':`content-${seq}`}});
it('groups adjacent successful reads without hiding a failure or crossing a user turn', () => {
  const projection = projectAgentTranscript([read(1,'a'),read(2,'a'),read(3,'a',true),{seq:4,run:'b',kind:'userMessage',text:'Next question',data:null},read(5,'b')]);
  const view = render(<AgentReplyTimeline {...projection} runStates={new Map()} engine="codex"/>);
  const group = view.container.querySelector('.agent-reply-read-group') as HTMLDetailsElement;
  expect(group.open).toBe(false);
  expect(group.querySelectorAll('[data-event-kind="commandExecution"]')).toHaveLength(2);
  expect(screen.getByText('agentExecution.failed').closest('.agent-reply-read-group')).toBeNull();
  expect(screen.getByText('Next question')).toBeTruthy();
  fireEvent.click(group.querySelector('summary')!);
  expect(group.open).toBe(true);
});
it('preserves a user-opened read group across a later appended answer', () => {
  const events = [read(1,'a'),read(2,'a')];
  const initial = projectAgentTranscript(events);
  const view = render(<AgentReplyTimeline {...initial} runStates={new Map()} engine="codex"/>);
  const group = view.container.querySelector('.agent-reply-read-group') as HTMLDetailsElement;
  fireEvent.click(group.querySelector('summary')!);
  const next = projectAgentTranscript([...events,{seq:3,run:'a',kind:'agentMessage',text:'Completed answer',data:{id:'answer',phase:'final_answer'}}]);
  view.rerender(<AgentReplyTimeline {...next} runStates={new Map()} engine="codex"/>);
  expect(view.container.querySelector('.agent-reply-read-group')).toBe(group);
  expect(group.open).toBe(true);
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

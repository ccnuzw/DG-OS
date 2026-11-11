import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { AgentEvent, Json } from '../src/stores/agentRunnerStore';
import { EventContent, NativeEvent } from '../src/components/development/AgentReplyContent';
import { AgentDiff, NativeValue } from '../src/components/development/AgentExecutionViews';
import { ToolDetails } from '../src/components/chat/ExecutionDetails';
import { CodeBlock } from '../src/components/chat/Markdown';
import { itemKey, projectAgentTranscript } from '../src/components/development/agentTranscript';
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string, args?: Record<string, unknown>) => args ? `${key} ${Object.values(args).join(' ')}` : key }), t: (key: string, args?: Record<string, unknown>) => args ? `${key} ${Object.values(args).join(' ')}` : key }));
afterEach(cleanup);
const event = (seq: number, kind: string, data: Json, text: string | null = null): AgentEvent => ({ seq, kind, data, text, run: 'run' });

it('keeps native tool errors visible while long output stays lazy and preserves user expansion on updates', async () => {
  const use = event(1, 'message', { type: 'assistant', content: [{ type: 'text', text: 'I will inspect the file.' }, { type: 'tool_use', id: 'read', name: 'Read', input: { file_path: 'src/app.ts', offset: 3, limit: 10 } }] });
  const results = new Map([[itemKey(use, 'read'), { type: 'tool_result', tool_use_id: 'read', is_error: true, content: 'Permission denied for src/app.ts' } as Json]]);
  const view = render(<EventContent event={use} toolResults={results} engine="claude" />);
  expect(screen.getByRole('alert').textContent).toContain('Permission denied');
  expect(screen.getByText('I will inspect the file.').closest('[role="alert"]')).toBeNull();
  expect(view.container.querySelector('[data-tool-view]')).toBeNull();
  const summary = screen.getByText('agentExecution.read').closest('summary')!;
  fireEvent.click(summary);
  expect(await screen.findByText('agentExecution.offset')).toBeTruthy();
  const details = summary.closest('details')!;
  expect(details.open).toBe(true);
  view.rerender(<EventContent event={{ ...use, seq: 2 }} toolResults={results} engine="claude" />);
  expect(details.open).toBe(true);
  expect(screen.getByText('agentExecution.limit')).toBeTruthy();
});

it('renders actual Claude TodoWrite steps and retains unknown statuses without inventing completion', async () => {
  const value = event(1, 'message', { type: 'assistant', content: [{ type: 'tool_use', id: 'plan', name: 'TodoWrite', input: { todos: [{ content: 'Inspect native output', status: 'completed' }, { content: 'Review result', status: 'in_progress' }, { content: 'Await native data', status: 'custom' }] } }] });
  const view = render(<EventContent event={value} runState="unknown" />);
  expect(screen.queryByText('Inspect native output')).toBeNull();
  fireEvent.click(screen.getByText('execution.plan'));
  expect(await screen.findByText('Inspect native output')).toBeTruthy();
  expect(screen.getByText('Review result').closest('li')?.getAttribute('data-plan-status')).toBe('in_progress');
  expect(screen.getByText('Await native data').closest('li')?.textContent).toContain('custom');
  expect(view.container.querySelector('[data-operation-state]')?.getAttribute('data-operation-state')).toBe('unknown');
});

it('keeps child messages folded under only their native parent and displays lifecycle status as reported', async () => {
  const value = event(1, 'message', { type: 'assistant', parent_tool_use_id: 'parent-42', content: [{ type: 'text', text: 'Child agent result' }] });
  const view = render(<EventContent event={value} engine="claude" />);
  expect(view.container.textContent).toContain('parent-42');
  expect(screen.queryByText('Child agent result')).toBeNull();
  fireEvent.click(screen.getByText('agentExecution.childMessage'));
  expect(await screen.findByText('Child agent result')).toBeTruthy();
  view.rerender(<EventContent event={event(2, 'task_notification', { task_id: 'task-1', status: 'completed', description: 'Inspect dependencies', summary: 'Native task summary' })} />);
  expect(screen.getByText('agentExecution.toolCompleted')).toBeTruthy();
  expect(screen.queryByText(/parent-42/)).toBeNull();
  fireEvent.click(screen.getByText('agentExecution.subtask'));
  expect(await screen.findByText('Native task summary')).toBeTruthy();
});

it('retains native phase semantics and engine identity without visual phase badges', () => {
  const view = render(<EventContent event={event(1, 'agentMessage', { phase: 'commentary' }, 'Inspecting dependencies')} engine="codex" />);
  expect(screen.getByTestId('agent-event-1').getAttribute('data-message-phase')).toBe('commentary');
  expect(screen.getByText('Inspecting dependencies').closest('[data-message-phase]')?.getAttribute('data-message-phase')).toBe('commentary');
  expect(screen.getByText('Codex')).toBeTruthy();
  expect(screen.queryByText('codexTranscript.commentary')).toBeNull();
  view.rerender(<EventContent event={event(1, 'agentMessage', { phase: 'final_answer' }, 'The dependency is fixed.')} engine="codex" />);
  expect(screen.getByTestId('agent-event-1').getAttribute('data-message-phase')).toBe('final_answer');
  expect(screen.getByText('Codex')).toBeTruthy();
  expect(screen.queryByText('codexTranscript.finalAnswer')).toBeNull();
  view.rerender(<EventContent event={event(1, 'agentMessage', { phase: 'future' }, 'Ordinary native text')} engine="codex" />);
  expect(screen.getByTestId('agent-event-1').hasAttribute('data-message-phase')).toBe(false);
  expect(screen.getByText('Ordinary native text')).toBeTruthy();
});

it.each(['user', 'commentary'] as const)('offers %s copy and quote through a visible keyboard menu with Escape focus return', async kind => {
  const source = 'Complete native message\nwith its second line retained.';
  const value = kind === 'user' ? event(1, 'userMessage', null, source) : event(1, 'agentMessage', { phase: 'commentary' }, source);
  const writeText = vi.fn().mockResolvedValue(undefined), onQuoteReply = vi.fn();
  const original = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  try {
    const view = render(<EventContent event={value} engine="codex" onQuoteReply={onQuoteReply} />);
    const actions = screen.getByRole('group', { name: 'agentReply.actions' });
    const trigger = within(actions).getByRole('button', { name: 'agentReply.actions' });
    expect(actions.getAttribute('data-compact')).toBe('true');
    expect(trigger.closest('.agent-reply-prose-heading')).toBeTruthy();
    expect(within(actions).getAllByRole('button')).toHaveLength(1);
    expect(screen.queryByRole('button', { name: 'agentReply.copy' })).toBeNull();
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'ArrowDown' });
    const copyItem = await screen.findByRole('menuitem', { name: 'agentReply.copy' });
    await waitFor(() => expect(document.activeElement).toBe(copyItem));
    fireEvent.keyDown(copyItem, { key: 'ArrowDown' });
    const quoteItem = screen.getByRole('menuitem', { name: 'agentReply.quote' });
    await waitFor(() => expect(document.activeElement).toBe(quoteItem));
    fireEvent.keyDown(quoteItem, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(trigger));
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(await screen.findByRole('menuitem', { name: 'agentReply.copy' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledExactlyOnceWith(source));
    expect(await within(actions).findByText('chat.copied')).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(await screen.findByRole('menuitem', { name: 'agentReply.quote' }));
    await waitFor(() => expect(onQuoteReply).toHaveBeenCalledExactlyOnceWith(source));
    expect(view.container.querySelector('.agent-reply-phase')).toBeNull();
  } finally {
    if (original) Object.defineProperty(navigator, 'clipboard', original);
    else Reflect.deleteProperty(navigator, 'clipboard');
  }
});

it.each(['final_answer', 'future', undefined])('keeps direct copy and quote visible for assistant phase %s', phase => {
  const source = 'The complete final answer.', onQuoteReply = vi.fn();
  render(<EventContent event={event(1, 'agentMessage', phase ? { phase } : null, source)} engine="codex" onQuoteReply={onQuoteReply} />);
  const actions = screen.getByRole('group', { name: 'agentReply.actions' });
  expect(actions.hasAttribute('data-compact')).toBe(false);
  expect(within(actions).getByRole('button', { name: 'agentReply.copy' })).toBeTruthy();
  fireEvent.click(within(actions).getByRole('button', { name: 'agentReply.quote' }));
  expect(onQuoteReply).toHaveBeenCalledExactlyOnceWith(source);
  expect(within(actions).queryByRole('button', { name: 'agentReply.actions' })).toBeNull();
});

it('renders a successful terminal as one short disclosure and keeps its original evidence available', async () => {
  const value = event(1, 'result', { run_status: 'completed', customEvidence: 'Preserved terminal evidence' });
  const original = JSON.stringify(value);
  const view = render(<EventContent event={value} engine="claude" />);
  const terminal = view.container.querySelector('.agent-reply-terminal') as HTMLDetailsElement;
  expect(terminal.getAttribute('data-operation-state')).toBe('replyCompleted');
  expect(terminal.open).toBe(false);
  expect(terminal.querySelector('summary')?.textContent).toBe('agentExecution.replyCompleted');
  expect(view.container.querySelector('.agent-reply-operation')).toBeNull();
  expect(screen.queryByText('agent.finished')).toBeNull();
  expect(screen.queryByText('agentExecution.notAcceptance')).toBeNull();
  expect(view.container.querySelector('pre')).toBeNull();
  fireEvent.click(screen.getByText('agentExecution.replyCompleted'));
  expect(await screen.findByText('agentExecution.notAcceptance')).toBeTruthy();
  fireEvent.click(screen.getByText(/agent.details/));
  expect(await screen.findByText(/Preserved terminal evidence/, { selector: 'pre' })).toBeTruthy();
  expect(JSON.stringify(value)).toBe(original);
});

it.each([['failed', 'failed'], ['cancelled', 'stopped'], ['unknown', 'unknown']] as const)('keeps a %s terminal state explicit', (run_status, status) => {
  const view = render(<EventContent event={event(1, 'result', { run_status })} runState="processing" />);
  expect(view.container.querySelector('.agent-reply-terminal')).toBeNull();
  expect(view.container.querySelector('.agent-reply-operation')?.getAttribute('data-operation-state')).toBe(status);
  expect(screen.getByText(`agentExecution.${status}`)).toBeTruthy();
  if (run_status === 'failed') expect(screen.getByRole('alert')).toBeTruthy();
});


it('reuses complete copying while folding only long code and retains a manual expansion', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  const original = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  try {
    const short = 'const answer = 42;';
    const view = render(<CodeBlock lang="typescript" text={short} />);
    expect(view.container.querySelector('pre code')?.textContent).toBe(short);
    expect(screen.queryByRole('button', { name: /agentExecution.expandCode/ })).toBeNull();
    const long = Array.from({ length: 30 }, (_, index) => `line ${index + 1}`).join('\n');
    view.rerender(<CodeBlock lang="text" text={long} />);
    expect(view.container.querySelector('pre code')?.textContent).not.toContain('line 30');
    fireEvent.click(screen.getByRole('button', { name: 'chat.copyCode' }));
    expect(writeText).toHaveBeenCalledWith(long);
    fireEvent.click(screen.getByRole('button', { name: /agentExecution.expandCode/ }));
    expect(view.container.querySelector('pre code')?.textContent).toBe(long);
    view.rerender(<CodeBlock lang="text" text={`${long}\nline 31`} />);
    expect(view.container.querySelector('pre code')?.textContent).toContain('line 31');
    expect(screen.getByRole('button', { name: /agentExecution.collapseCode/ }).getAttribute('aria-expanded')).toBe('true');
  } finally {
    if (original) Object.defineProperty(navigator, 'clipboard', original);
    else Reflect.deleteProperty(navigator, 'clipboard');
  }
});

it('renders unified diff line numbers and signs with separate native and disk provenance', () => {
  const diff = '--- a/app.ts\n+++ b/app.ts\n@@ -7,2 +7,2 @@\n-old\n+new\n same';
  const view = render(<AgentDiff diff={diff} seq={6} />);
  const deleted = screen.getByText('-old').closest('[data-before-line]')!;
  expect(deleted.getAttribute('data-before-line')).toBe('7');
  expect(deleted.hasAttribute('data-after-line')).toBe(false);
  const added = screen.getByText('+new').closest('[data-after-line]')!;
  expect(added.getAttribute('data-after-line')).toBe('7');
  expect(added.hasAttribute('data-before-line')).toBe(false);
  expect(view.container.querySelector('[data-diff-source]')?.getAttribute('data-diff-source')).toBe('native');
  view.rerender(<AgentDiff diff={diff} source="disk" />);
  expect(view.container.querySelector('[data-diff-source]')?.getAttribute('data-diff-source')).toBe('disk');
  expect(screen.queryByRole('button', { name: /apply|rollback|restore/i })).toBeNull();
});

it.each([
  ['Grep', { pattern: 'native', path: 'src', output_mode: 'content' }, 'native match'],
  ['Glob', { pattern: '**/*.ts', path: 'src' }, 'src/app.ts'],
  ['Bash', { command: 'printf native', cwd: '/project' }, 'native stdout'],
  ['Edit', { file_path: 'src/app.ts', old_string: 'before edit', new_string: 'after edit' }, 'edited'],
  ['Write', { file_path: 'src/new.ts', content: 'const created = true;' }, 'written'],
  ['mcp__files__inspect', { resource: 'native-resource' }, { native_result: 'preserved' }],
] as const)('preserves actual %s input and output in its dedicated view', async (name, input, output) => {
  const view = render(<ToolDetails name={name} input={input as Json} output={output as Json} />);
  if (typeof output === 'string') expect(view.container.textContent).toContain(output);
  else { fireEvent.click(screen.getByText('agentResources.fields')); expect(await screen.findByText('preserved')).toBeTruthy(); }
  if (name === 'Edit') { expect(view.container.textContent).toContain('before edit'); expect(view.container.textContent).toContain('after edit'); }
  if (name === 'Write') expect(view.container.textContent).toContain('const created = true;');
  if (name === 'Bash') expect(view.container.textContent).toContain('printf native');
});

it('retains unknown native fields and inspects true source events while suppressing private thought blocks', async () => {
  const value = event(8, 'message', { type: 'assistant', content: [{ type: 'text', text: 'Public answer' }, { type: 'thinking', thinking: 'PRIVATE', signature: 'SECRET' }, { type: 'redacted_thinking', data: 'REDACTED' }] });
  const clone = { ...value, data: { content: [{ type: 'text', text: 'Cloned presentation' }] } };
  const view = render(<><NativeEvent event={clone} sources={[value]} /><NativeValue value={{ custom: { field: 'unknown-native-field' }, content: [{ type: 'thinking', thinking: 'PRIVATE' }] }} /></>);
  expect(screen.getByText('unknown-native-field')).toBeTruthy();
  fireEvent.click(screen.getByText(/agent.details/));
  expect(await screen.findByText(/Public answer/)).toBeTruthy();
  expect(view.container.textContent).not.toMatch(/PRIVATE|SECRET|REDACTED|Cloned presentation/);
});

it('opens current disk changes only through the existing explicit reference callback', () => {
  const onOpenReference = vi.fn();
  render(<EventContent event={event(1, 'fileChange', { type: 'fileChange', changes: [{ path: 'src/app.ts', diff: '@@ -1 +1 @@\n-old\n+new' }] })} onOpenReference={onOpenReference} />);
  expect(onOpenReference).not.toHaveBeenCalled();
  const button = screen.getByRole('button', { name: 'agent.openDiff' });
  expect(button.getAttribute('title')).toBe('agentExecution.diskDiff');
  fireEvent.click(button);
  expect(onOpenReference).toHaveBeenCalledWith({ path: 'src/app.ts', source: 'fileChange', diff: true }, 'diff');
});

it('keeps each native diff file header separate and preserves change signs inside content', () => {
  const diff = 'diff --git a/one b/one\n--- a/one\n+++ b/one\n@@ -1 +1 @@\n---old content\n+++new content\ndiff --git a/two b/two\n--- a/two\n+++ b/two\n@@ -10 +20 @@\n-old second\n+new second';
  render(<AgentDiff diff={diff} />);
  expect(screen.getByText('--- a/two').closest('[data-before-line]')).toBeNull();
  expect(screen.getByText('+++ b/two').closest('[data-after-line]')).toBeNull();
  expect(screen.getByText('---old content').closest('[data-before-line]')?.getAttribute('data-before-line')).toBe('1');
  expect(screen.getByText('+++new content').closest('[data-after-line]')?.getAttribute('data-after-line')).toBe('1');
  expect(screen.getByText('-old second').closest('[data-before-line]')?.getAttribute('data-before-line')).toBe('10');
  expect(screen.getByText('+new second').closest('[data-after-line]')?.getAttribute('data-after-line')).toBe('20');
});

it('tolerates missing native fields without creating completion evidence or parent attribution', () => {
  render(<><EventContent event={event(1, 'item/started', { type: 'commandExecution' })} /><EventContent event={event(2, 'task_notification', null)} /><EventContent event={event(3, 'message', { type: 'assistant', content: [{ type: 'tool_use', name: 'UnknownTool' }] })} /></>);
  expect(screen.queryByText('agentExecution.toolCompleted')).toBeNull();
  expect(screen.queryByText('agentExecution.replyCompleted')).toBeNull();
  expect(screen.queryByText(/agentExecution.parentTool/)).toBeNull();
  expect(within(screen.getByTestId('agent-event-1')).getByText('agentExecution.processing')).toBeTruthy();
});

it('shows a native declined operation without inheriting the active run status', () => {
  const value = event(1, 'commandExecution', { type: 'commandExecution', status: 'declined', command: 'native command' });
  render(<EventContent event={value} runState="processing" />);
  expect(screen.getByText('agentExecution.declined')).toBeTruthy();
  expect(screen.queryByText('agentExecution.processing')).toBeNull();
  expect(screen.queryByText('agentExecution.toolCompleted')).toBeNull();
});

it('shows complete streamed command output and scopes elapsed progress to each native tool', async () => {
  const command = event(1, 'commandExecution', { type: 'commandExecution', status: 'completed', aggregatedOutput: 'retained stdout\nretained stderr', exitCode: 1 });
  const view = render(<EventContent event={command} />);
  expect(screen.getByRole('alert').textContent).toContain('retained stdout');
  fireEvent.click(view.container.querySelector('summary')!);
  expect(await screen.findByText(/retained stderr/, { selector: 'code' })).toBeTruthy();
  view.rerender(<EventContent event={event(2, 'message', { type: 'assistant', content: [{ type: 'tool_use', id: 'one', name: 'Read', input: { file_path: 'one.ts' } }, { type: 'tool_use', id: 'two', name: 'Read', input: { file_path: 'two.ts' } }], tool_progress: { one: { elapsed_time_seconds: 3 }, two: { elapsed_time_seconds: 8 } } })} />);
  for (const summary of view.container.querySelectorAll('.agent-reply-operation > summary')) fireEvent.click(summary);
  const operations = view.container.querySelectorAll('.agent-reply-operation');
  await waitFor(() => expect(operations[0].textContent).toContain('execution.elapsed 3'));
  expect(operations[0].textContent).not.toContain('execution.elapsed 8');
  expect(operations[1].textContent).toContain('execution.elapsed 8');
});

it.each([false, true])('retains a merged Bash result diagnostic on row zero and navigates its actual failure (%s)', async failed => {
  const use = event(1, 'message', { type: 'assistant', native_message_id: 'native-tool-message', block_indices: [0], content: [{ type: 'tool_use', id: 'bash', name: 'Bash', input: { command: 'compile' } }] });
  const result = event(2, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'bash', is_error: failed, content: '/repo/src/a.ts:12:3: warning: native diagnostic' }] });
  const projection = projectAgentTranscript([use, result]);
  expect(projection.rows).toHaveLength(1);
  expect(projection.rows[0].sources).toEqual([use, result]);
  const onOpenReference = vi.fn(), onSelectRun = vi.fn();
  const view = render(<EventContent {...projection.rows[0]} toolResults={projection.toolResults} toolNames={projection.toolNames} engine="claude" onOpenReference={onOpenReference} onSelectRun={onSelectRun} />);
  expect(onOpenReference).not.toHaveBeenCalled();
  expect(view.container.querySelector('[data-operation-state]')?.getAttribute('data-operation-state')).toBe(failed ? 'failed' : 'toolCompleted');
  expect(screen.queryByRole('button', { name: 'agent.openFile' })).toBeNull();
  fireEvent.click(screen.getByText('agent.references'));
  const references = await screen.findAllByRole('button', { name: 'agent.openFile' });
  expect(references).toHaveLength(1);
  expect(view.container.querySelectorAll('[data-reference-path]')).toHaveLength(1);
  fireEvent.click(references[0]);
  expect(onOpenReference).toHaveBeenCalledWith({ path: '/repo/src/a.ts', line: 12, source: 'diagnostic', diff: false }, 'file');
  if (failed) {
    fireEvent.click(screen.getByRole('button', { name: 'agent.openRun' }));
    expect(onSelectRun).toHaveBeenCalledExactlyOnceWith('run');
  } else expect(screen.queryByRole('button', { name: 'agent.openRun' })).toBeNull();
});

it('keeps native result stderr diagnostics after merging without parsing another tool lane or dialogue', async () => {
  const use = event(1, 'message', { type: 'assistant', content: [{ type: 'text', text: '/repo/fake.ts:1:1: error: assistant prose' }, { type: 'tool_use', id: 'bash', name: 'Bash', input: { command: 'compile' } }, { type: 'tool_use', id: 'other', name: 'Grep', input: { pattern: 'error' } }] });
  const result = event(2, 'message', { type: 'user', tool_use_result: { stderr: '/repo/src/a.ts:12:3: warning: native stderr' }, content: [{ type: 'tool_result', tool_use_id: 'bash', content: '' }] });
  const other = event(3, 'message', { type: 'user', content: [{ type: 'tool_result', tool_use_id: 'other', content: '/repo/other.ts:6:2: error: search text' }] });
  const projection = projectAgentTranscript([use, result, other]);
  const view = render(<EventContent {...projection.rows[0]} toolResults={projection.toolResults} toolNames={projection.toolNames} onOpenReference={vi.fn()} />);
  fireEvent.click(screen.getByText('agent.references'));
  expect(await screen.findByRole('button', { name: 'agent.openFile' })).toBeTruthy();
  expect(Array.from(view.container.querySelectorAll('[data-reference-path]'), node => node.getAttribute('data-reference-path'))).toEqual(['/repo/src/a.ts']);
});

it('omits routine and unknown native notifications even when content is rendered directly', () => {
  const view = render(<><EventContent event={event(1, 'native/unsupported', { native_type: 'hook/completed', payload_omitted: true })} runState="processing" /><EventContent event={event(2, 'native/unsupported', { native_type: 'future/event', status: 'completed', success: true })} runState="stopped" /></>);
  expect(view.container.childElementCount).toBe(0);
  expect(screen.queryByText('agentExecution.unknown')).toBeNull();
  expect(screen.queryByText('agentExecution.toolCompleted')).toBeNull();
});

it('keeps a warning without retained detail to one summary and lazily exposes its available evidence', async () => {
  const warning = event(1, 'native/unsupported', { native_type: 'warning', diagnostic: 'unsupported_native_event', payload_omitted: true });
  const view = render(<EventContent event={warning} runState="processing" engine="codex" />);
  expect(screen.getByRole('alert').textContent).toContain('agentReply.nativeWarning');
  expect(screen.getByRole('alert').textContent).toContain('agentReply.nativeWarningNoMessage');
  expect(screen.queryByText('agentReply.nativeWarningHelp')).toBeNull();
  expect(view.container.querySelector('[data-operation-state]')).toBeNull();
  expect(view.container.querySelector('[data-native-severity]')?.getAttribute('data-native-severity')).toBe('warning');
  expect(view.container.querySelectorAll('details')).toHaveLength(1);
  expect(view.container.querySelector('details > summary [role="alert"]')).toBeTruthy();
  expect(screen.queryByText('agentExecution.unknown')).toBeNull();
  expect(view.container.querySelector('pre')).toBeNull();
  fireEvent.click(screen.getByText('agent.details'));
  expect(await screen.findByText(/unsupported_native_event/, { selector: 'pre' })).toBeTruthy();
  expect(screen.getByText('agentReply.nativeWarningHelp')).toBeTruthy();
});

it.each([
  { native_type: 'error' },
  { native_type: 'hook/failed' },
  { native_type: 'future/event', error: 'Native failure message' },
  { native_type: 'future/event', errors: ['Native failure message'] },
  { native_type: 'mcpServer/startupStatus/updated', status: 'failed' },
  { native_type: 'future/event', status: 'error' },
  { native_type: 'warning', success: false },
] as Json[])('keeps explicit diagnostic failures visible independently of the active run: %j', data => {
  const view = render(<EventContent event={event(1, 'native/unsupported', data)} runState="processing" />);
  expect(screen.getByRole('alert').textContent).toContain('agentReply.nativeFailure');
  expect(view.container.querySelector('[data-native-severity]')?.getAttribute('data-native-severity')).toBe('failed');
  expect(screen.queryByText('agentExecution.unknown')).toBeNull();
  expect(screen.queryByText('agentExecution.processing')).toBeNull();
});

it('preserves retained warning text while keeping private native blocks out of the visible notice', () => {
  const warning = event(1, 'native/unsupported', { native_type: 'warning', message: 'A native configuration needs attention.', content: [{ type: 'thinking', thinking: 'PRIVATE_THINKING' }] }, 'PRIVATE_TEXT');
  const original = JSON.stringify(warning);
  const view = render(<EventContent event={warning} />);
  expect(screen.getByRole('alert').textContent).toContain('A native configuration needs attention.');
  expect(screen.getByRole('alert').closest('summary')).toBeNull();
  expect(view.container.querySelector('.agent-reply-native-notice-compact')).toBeNull();
  expect(screen.queryByText('agentReply.nativeWarningNoMessage')).toBeNull();
  expect(view.container.textContent).not.toMatch(/PRIVATE_/);
  expect(JSON.stringify(warning)).toBe(original);
});

it('quotes from a compact menu only after closing and leaves focus in the composer', async () => {
  const source = 'Retained complete commentary';
  const onQuoteReply = vi.fn(() => screen.getByRole('textbox', { name: 'Composer' }).focus());
  render(<><EventContent event={event(1, 'agentMessage', { phase: 'commentary' }, source)} engine="codex" onQuoteReply={onQuoteReply} /><textarea aria-label="Composer" /></>);
  const trigger = screen.getByRole('button', { name: 'agentReply.actions' });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'ArrowDown' });
  const quote = await screen.findByRole('menuitem', { name: 'agentReply.quote' });
  fireEvent.click(quote);
  await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Composer' })));
  expect(onQuoteReply).toHaveBeenCalledExactlyOnceWith(source);
});

it.each(['waiting', 'pending', 'queued'])('retains native %s status despite a stale zero exit code and another run state', status => {
  const view = render(<EventContent event={event(1, 'commandExecution', { type: 'commandExecution', status, exitCode: 0 })} runState="processing" />);
  expect(view.container.querySelector('[data-operation-state]')?.getAttribute('data-operation-state')).toBe('waiting');
  expect(screen.queryByText('agentExecution.toolCompleted')).toBeNull();
});

it('does not convert native in-progress evidence with a zero exit code into completion', () => {
  const view = render(<EventContent event={event(1, 'commandExecution', { type: 'commandExecution', status: 'inProgress', exitCode: 0 })} runState="waiting" />);
  expect(view.container.querySelector('[data-operation-state]')?.getAttribute('data-operation-state')).toBe('waiting');
  expect(screen.queryByText('agentExecution.toolCompleted')).toBeNull();
});

it.each([
  { errors: ['Retained failure'] },
  { status: 'error', error: 'Retained failure' },
  { result: { is_error: true, content: 'Retained failure' } },
] as Json[])('keeps empty assistant failures visible: %j', data => {
  const value = event(1, 'agentMessage', { type: 'assistant', ...data as Record<string, Json> });
  const projection = projectAgentTranscript([value]);
  expect(projection.rows).toHaveLength(1);
  const view = render(<EventContent {...projection.rows[0]} />);
  expect(screen.getByRole('alert').textContent).toContain('Retained failure');
  expect(view.container.querySelector('[data-operation-state]')?.getAttribute('data-operation-state')).toBe('failed');
});

it.each(['failed', 'waiting'] as const)('exposes %s child tool status while retaining its collapsed evidence', state => {
  const value = event(1, 'message', { type: 'assistant', parent_tool_use_id: 'parent', content: [{ type: 'tool_use', id: 'read', name: 'Read', input: { file_path: 'child.ts' } }] });
  const result: Json = { type: 'tool_result', tool_use_id: 'read', ...(state === 'failed' ? { isError: true, content: 'Child permission denied' } : { status: 'pending' }) };
  const view = render(<EventContent event={value} toolResults={new Map([[itemKey(value, 'read'), result]])} runState="processing" />);
  const child = view.container.querySelector('.agent-reply-subtask-message') as HTMLDetailsElement;
  expect(child.open).toBe(false);
  expect(child.querySelector('summary')?.textContent).toContain(`agentExecution.${state}`);
  expect(child.getAttribute('data-operation-state')).toBe(state);
  expect(screen.queryByText('Child permission denied')).toBeNull();
});

it('renders one failed native activity operation and keeps its failure outside the disclosure', () => {
  const view = render(<EventContent event={event(1, 'collabAgentToolCall', { type: 'collabAgentToolCall', status: 'failed', error: 'Native spawn failed' })} />);
  expect(view.container.querySelectorAll('.agent-reply-operation')).toHaveLength(1);
  expect(screen.getByRole('alert').textContent).toContain('Native spawn failed');
});

it.each(['unknown', 'cancelled'] as const)('retains authoritative terminal %s despite native error flags', run_status => {
  const view = render(<EventContent event={event(1, 'result', { run_status, is_error: true, terminal_reason: run_status === 'cancelled' ? 'aborted_tools' : 'future_native_reason' })} runState="processing" />);
  const expected = run_status === 'cancelled' ? 'stopped' : 'unknown';
  expect(view.container.querySelector('[data-operation-state]')?.getAttribute('data-operation-state')).toBe(expected);
  expect(screen.queryByText('agentExecution.failed')).toBeNull();
});

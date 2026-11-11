import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { AgentOutputText } from '../src/components/development/AgentOutputText';
import ExecutionDetails, { ToolDetails } from '../src/components/chat/ExecutionDetails';
import type { AgentEvent, Json } from '../src/stores/agentRunnerStore';

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
vi.mock('../src/i18n', () => ({
  useLocale: () => ({ locale: 'en-US', t: (key: string, args?: Record<string, unknown>) => args ? `${key} ${Object.values(args).join(' ')}` : key }),
  t: (key: string, args?: Record<string, unknown>) => args ? `${key} ${Object.values(args).join(' ')}` : key,
}));
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it('previews only eight lines of a normal 45-line tool response until explicitly expanded', () => {
  const text = Array.from({ length: 45 }, (_, index) => `line-${index + 1}`).join('\n');
  const view = render(<AgentOutputText text={text} lang="typescript" />);
  const output = view.container.querySelector('.agent-reply-output-preview')!;
  expect(output.textContent).toBe(text.split('\n').slice(0, 8).join('\n'));
  expect(output.textContent).not.toContain('line-9');
  expect(screen.getByRole('textbox', { name: 'agentReply.searchOutput' })).toBeTruthy();
  const expand = screen.getByRole('button', { name: 'agentReply.expandOutput' });
  expect(expand.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(expand);
  expect(output.textContent).toBe(text);
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.collapseOutput' }));
  expect(output.textContent).not.toContain('line-9');
});

it('bounds character previews, keeps a fixed expansion cap and copies the complete native output', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  const text = `${'a'.repeat(30000)}complete native ending`;
  const view = render(<AgentOutputText text={text} />);
  const output = view.container.querySelector('.agent-reply-output-preview')!;
  expect(output.textContent).toHaveLength(1600);
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.expandOutput' }));
  expect(output.textContent).toHaveLength(24000);
  expect(screen.getByText('agentReply.outputSafetyLimit 24000')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.copyOutput' }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(text));
});

it('finds and locates matching native lines beyond both preview limits without changing full-output copy', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  const text = Array.from({ length: 140 }, (_, index) => index === 100 || index === 125 ? `line ${index + 1}: Needle evidence` : `line ${index + 1}: ${'x'.repeat(260)}`).join('\n');
  const view = render(<AgentOutputText text={text} />);
  const output = view.container.querySelector('.agent-reply-output-preview') as HTMLPreElement;
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: 'needle' } });
  expect(screen.getByText('agentReply.matchCount 2')).toBeTruthy();
  expect(output.textContent).not.toContain('Needle');
  fireEvent.click(view.container.querySelector('.agent-reply-output-match')!);
  expect(output.dataset.outputStartLine).toBe('101');
  expect(output.querySelector('mark')?.textContent).toBe('Needle');
  expect(document.activeElement).toBe(output);
  expect(output.textContent!.split('\n').length).toBeLessThanOrEqual(8);
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.nextOutputMatch' }));
  expect(output.dataset.outputStartLine).toBe('126');
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.previousOutputMatch' }));
  expect(output.dataset.outputStartLine).toBe('101');
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.copyOutput' }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(text));
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: 'absent marker' } });
  expect(screen.getByText('agentReply.noMatch 0')).toBeTruthy();
  expect(output.dataset.outputStartLine).toBe('1');
  expect(view.container.querySelector('.agent-reply-output-match')).toBeNull();
});

it('locates a match deep inside one long native line using a bounded display window', () => {
  const text = `${'x'.repeat(50000)}Needle${'y'.repeat(2000)}`;
  const view = render(<AgentOutputText text={text} />);
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: 'Needle' } });
  fireEvent.click(view.container.querySelector('.agent-reply-output-match')!);
  const output = view.container.querySelector('.agent-reply-output-preview')!;
  expect(output.textContent).toHaveLength(1600);
  expect(output.querySelector('mark')?.textContent).toBe('Needle');
});

it.each([
  ['literal Unicode text', 'x'.repeat(50000) + '路径.[🧪] Καλημέρα', '路径.[🧪]'],
  ['Unicode case folding', 'x'.repeat(50000) + 'ſource KELVIN', 'SOURCE kelvin'],
])('finds %s with native offsets beyond the display cap', (_, text, query) => {
  const view = render(<AgentOutputText text={text} />);
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: query } });
  expect(screen.getByText('agentReply.matchCount 1')).toBeTruthy();
  fireEvent.click(view.container.querySelector('.agent-reply-output-match')!);
  const output = view.container.querySelector('.agent-reply-output-preview')!;
  expect(output.querySelector('mark')?.textContent).toBe(text.slice(50000, 50000 + query.length));
  expect(output.textContent!.length).toBeLessThanOrEqual(24000);
});

it('never cuts a Unicode scalar at preview, expansion, match-window or result-snippet boundaries', () => {
  const text = `${'a'.repeat(1599)}🧪${'b'.repeat(22398)}🧪${'c'.repeat(25000)}🧪${'d'.repeat(799)}Needle${'e'.repeat(798)}🧪tail`;
  const view = render(<AgentOutputText text={text} />);
  const output = view.container.querySelector('.agent-reply-output-preview')!;
  expect(output.textContent).toBe('a'.repeat(1599));
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.expandOutput' }));
  expect(output.textContent).toHaveLength(23999);
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: 'Needle' } });
  fireEvent.click(view.container.querySelector('.agent-reply-output-match')!);
  expect(output.textContent).toBe(`${'d'.repeat(799)}Needle${'e'.repeat(795)}`);
  expect(output.querySelector('mark')?.textContent).toBe('Needle');
  view.rerender(<AgentOutputText text={`${'x'.repeat(50000)}🧪${'y'.repeat(199)}Needle${'z'.repeat(193)}🧪tail`} />);
  const snippet = view.container.querySelector('.agent-reply-output-match code')!;
  expect(snippet.textContent).toBe(`${'y'.repeat(199)}Needle${'z'.repeat(193)}`);
});

it.each([24000, 30000])('keeps a %i-character search query visible within the fixed cap', length => {
  const query = 'q'.repeat(length);
  const text = `${'x'.repeat(50000)}${query}tail`;
  const view = render(<AgentOutputText text={text} />);
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: query } });
  fireEvent.click(view.container.querySelector('.agent-reply-output-match')!);
  const output = view.container.querySelector('.agent-reply-output-preview')!;
  expect(output.textContent).toBe('q'.repeat(24000));
  expect(output.querySelector('mark')?.textContent).toBe('q'.repeat(24000));
  expect(screen.getByText('agentReply.outputSafetyLimit 24000')).toBeTruthy();
});

it('retains located search and expanded state for appended streaming text', () => {
  const text = `${'x'.repeat(50000)}Needle`;
  const view = render(<AgentOutputText text={text} />);
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: 'Needle' } });
  fireEvent.click(view.container.querySelector('.agent-reply-output-match')!);
  view.rerender(<AgentOutputText text={`${text} appended tail`} />);
  const output = view.container.querySelector('.agent-reply-output-preview')!;
  expect(output.textContent).toContain('Needle appended tail');
  expect(output.querySelector('mark')?.textContent).toBe('Needle');
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.collapseOutput' }));
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.expandOutput' }));
  view.rerender(<AgentOutputText text={`${text} appended tail\nnext line`} />);
  expect(screen.getByRole('button', { name: 'agentReply.collapseOutput' }).getAttribute('aria-expanded')).toBe('true');
  expect(output.textContent).toHaveLength(24000);
});

it('resets stale locations and navigation when a completed output replaces a stream', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  const stream = Array.from({ length: 30 }, (_, index) => `line ${index + 1}: ${index === 10 || index === 25 ? 'Needle' : 'stream'}`).join('\n');
  const view = render(<AgentOutputText text={stream} />);
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: 'Needle' } });
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.nextOutputMatch' }));
  const output = view.container.querySelector('.agent-reply-output-preview')!;
  expect(output.getAttribute('data-output-start-line')).toBe('26');
  const replacement = Array.from({ length: 20 }, (_, index) => `final ${index + 1}: ${index === 15 ? 'Needle' : 'native'}`).join('\n');
  view.rerender(<AgentOutputText text={replacement} />);
  expect(output.textContent).toBe(replacement.split('\n').slice(0, 8).join('\n'));
  expect(screen.getByText('agentReply.outputMatchPosition 1 1')).toBeTruthy();
  expect((screen.getByRole('button', { name: 'agentReply.previousOutputMatch' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(view.container.querySelector('.agent-reply-output-match')!);
  expect(output.getAttribute('data-output-start-line')).toBe('16');
  view.rerender(<AgentOutputText text="short final output" />);
  expect(output.textContent).toBe('short final output');
  expect(screen.getByRole('textbox', { name: 'agentReply.searchOutput' })).toBeTruthy();
  expect(screen.getByText('agentReply.noMatch 0')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.copyOutput' }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith('short final output'));
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: '' } });
  expect(screen.queryByRole('textbox', { name: 'agentReply.searchOutput' })).toBeNull();
});

it('collapses a replacement even when the previous stream was expanded', () => {
  const view = render(<AgentOutputText text={'x'.repeat(30000)} />);
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.expandOutput' }));
  view.rerender(<AgentOutputText text={'y'.repeat(40000)} />);
  expect(view.container.querySelector('.agent-reply-output-preview')?.textContent).toBe('y'.repeat(1600));
  expect(screen.getByRole('button', { name: 'agentReply.expandOutput' }).getAttribute('aria-expanded')).toBe('false');
});

it('keeps an empty authoritative replacement empty and copies that exact output', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  const view = render(<AgentOutputText text={`${'x'.repeat(50000)}Needle`} />);
  fireEvent.change(screen.getByRole('textbox', { name: 'agentReply.searchOutput' }), { target: { value: 'Needle' } });
  fireEvent.click(view.container.querySelector('.agent-reply-output-match')!);
  view.rerender(<AgentOutputText text="" />);
  expect(view.container.querySelector('.agent-reply-output-preview')?.textContent).toBe('');
  expect(screen.queryByRole('button', { name: 'agentReply.collapseOutput' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.copyOutput' }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(''));
});

it.each(['Bash', 'commandExecution'])('renders a %s command exactly once and keeps other native input available', async name => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  const command = 'printf "complete command" && cat /work/source.ts';
  const view = render(<ToolDetails name={name} input={{ command, cwd: '/work', description: 'Read source', timeout: 1000 }} output="Native output" />);
  expect(view.container.querySelectorAll('.chat-code-block')).toHaveLength(1);
  expect(view.container.textContent?.split(command)).toHaveLength(2);
  fireEvent.click(screen.getByText('agentExecution.input'));
  await waitFor(() => expect(screen.getByText('1000')).toBeTruthy());
  expect(view.container.textContent?.split(command)).toHaveLength(2);
  expect(view.container.textContent).toContain('/work');
  expect(view.container.textContent).toContain('Read source');
  fireEvent.click(screen.getByRole('button', { name: 'chat.copyCode' }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(command));
});

it.each(['Bash', 'commandExecution'])('copies a long multiline %s command without truncation or duplicate input', async name => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  const command = `  printf '🧪'\n${Array.from({ length: 30 }, (_, index) => `printf 'line ${index + 1}'`).join('\n')}\n  `;
  const view = render(<ToolDetails name={name} input={{ command }} />);
  expect(view.container.querySelectorAll('.chat-code-block')).toHaveLength(1);
  expect(screen.queryByText('agentExecution.input')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'chat.copyCode' }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(command));
  fireEvent.click(screen.getByRole('button', { name: /agentExecution.expandCode/ }));
  expect(view.container.querySelector('.chat-code-block pre')?.textContent).toBe(command);
});

it.each([{ input: 'raw command input' }, { input: ['bash', '-lc', 'raw array command'] }])('retains non-object command input $input', async ({ input }) => {
  const view = render(<ToolDetails name="Bash" input={input} />);
  fireEvent.click(screen.getByText('agentExecution.input'));
  await waitFor(() => expect(view.container.textContent).toContain(Array.isArray(input) ? 'raw array command' : input));
});

it('retains structured command and working-directory values without duplicating the native input', async () => {
  const view = render(<ToolDetails name="commandExecution" input={{ command: ['bash', '-lc', 'native array command'], cwd: { path: '/native/work' }, timeout: 42 }} />);
  expect(view.container.textContent?.split('native array command')).toHaveLength(2);
  expect(view.container.textContent).toContain('/native/work');
  fireEvent.click(screen.getByText('agentExecution.input'));
  await waitFor(() => expect(screen.getByText('42')).toBeTruthy());
  expect(view.container.textContent?.split('native array command')).toHaveLength(2);
  expect(view.container.textContent?.split('/native/work')).toHaveLength(2);
});

it.each([{}, { cwd: null }])('omits missing command metadata without rendering an empty cwd line: %j', data => {
  const event: AgentEvent = { seq: 1, run: 'a', kind: 'commandExecution', text: null, data: { type: 'commandExecution', command: 'native command', ...data } };
  const view = render(<ExecutionDetails event={event} />);
  expect(view.container.textContent).not.toContain('agentExecution.cwd');
  expect(screen.queryByText('agentExecution.input')).toBeNull();
  expect(view.container.querySelector('.chat-code-block pre')?.textContent).toBe('native command');
});

it('preserves command arguments and descriptions through shared execution details', async () => {
  const event: AgentEvent = { seq: 1, run: 'a', kind: 'commandExecution', text: null, data: { type: 'commandExecution', input: { command: 'native nested command', cwd: '/native/work', timeout: 42 }, description: 'Native description' } };
  const view = render(<ExecutionDetails event={event} />);
  expect(view.container.querySelector('.chat-code-block pre')?.textContent).toBe('native nested command');
  expect(view.container.textContent).toContain('/native/work');
  expect(view.container.textContent).toContain('Native description');
  fireEvent.click(screen.getByText('agentExecution.input'));
  await waitFor(() => expect(screen.getByText('42')).toBeTruthy());
  expect(view.container.textContent?.split('native nested command')).toHaveLength(2);
});

it('applies the same bounded output behavior through shared execution details', () => {
  const text = Array.from({ length: 45 }, (_, index) => `native line ${index + 1}`).join('\n');
  const event: AgentEvent = { seq: 1, run: 'a', kind: 'commandExecution', text: null, data: { type: 'commandExecution', command: 'cat source.ts', cwd: '/work', aggregatedOutput: text, exitCode: 0 } };
  const view = render(<ExecutionDetails event={event} />);
  expect(view.container.textContent?.split('cat source.ts')).toHaveLength(2);
  expect(view.container.querySelector('.agent-reply-output-preview')?.textContent).toBe(text.split('\n').slice(0, 8).join('\n'));
  expect(screen.queryByText('agentExecution.input')).toBeNull();
});

it.each(['native string array', 'standalone text block', 'nested tool results'])('bounds and copies complete text from %s', async shape => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  const text = `${'x'.repeat(30000)}完整原文🧪`;
  const output: Json = shape === 'native string array' ? [text] : shape === 'standalone text block' ? { type: 'text', text } : { type: 'tool_result', content: [{ type: 'tool_result', content: [{ type: 'text', text }] }] };
  const view = render(<ToolDetails name="Bash" output={output} />);
  expect(view.container.querySelector('.agent-reply-output-preview')?.textContent).toBe('x'.repeat(1600));
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.expandOutput' }));
  expect(view.container.querySelector('.agent-reply-output-preview')?.textContent).toBe('x'.repeat(24000));
  fireEvent.click(screen.getByRole('button', { name: 'agentReply.copyOutput' }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(text));
});

it('keeps authoritative empty output from falling back to stale event text', () => {
  const event: AgentEvent = { seq: 1, run: 'a', kind: 'commandExecution', text: 'stale streamed output', data: { type: 'commandExecution', aggregatedOutput: '' } };
  const view = render(<ExecutionDetails event={event} />);
  expect(view.container.textContent).not.toContain('stale streamed output');
  expect(view.container.querySelector('.agent-reply-output-preview')).toBeNull();
});

it.each([{ nativeInput: 'native raw command' }, { nativeInput: ['bash', '-lc', 'native raw command'] }])('preserves shared native command input $nativeInput', async ({ nativeInput }) => {
  const event: AgentEvent = { seq: 1, run: 'a', kind: 'commandExecution', text: null, data: { type: 'commandExecution', input: nativeInput } };
  const view = render(<ExecutionDetails event={event} />);
  fireEvent.click(screen.getByText('agentExecution.input'));
  await waitFor(() => expect(view.container.textContent).toContain('native raw command'));
});

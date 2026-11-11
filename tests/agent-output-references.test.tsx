import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { AgentConversation } from '../src/pages/workbench/AgentConversation';
import { EventContent } from '../src/components/development/AgentReplyContent';
import { agentEventFailed, agentOutputReferences, resolveDevelopmentFileReference } from '../src/components/development/eventOutputReferences';
import { itemKey } from '../src/components/development/agentTranscript';
import { useAgentRunner, type AgentEvent, type AgentSession } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
afterEach(() => { cleanup(); invoke.mockReset(); useAgentRunner.getState().reset(); useDevelopment.getState().reset(); });
const event = (data: AgentEvent['data'], kind = 'fileChange'): AgentEvent => ({ seq: 1, kind, data, text: null, run: 'actual-run' });

it('uses Codex structured changes and Claude built-in file tool inputs', () => {
  expect(agentOutputReferences(event({ type: 'fileChange', changes: [{ path: '/task/src/app.ts', diff: 'patch' }] })).references).toEqual([{ path: '/task/src/app.ts', source: 'fileChange', diff: true }]);
  expect(agentOutputReferences(event({ type: 'assistant', content: [{ type: 'tool_use', name: 'Read', input: { file_path: 'src/app.ts', offset: 8 } }] }, 'message')).references).toEqual([{ path: 'src/app.ts', line: 8, source: 'tool', diff: false }]);
  expect(agentOutputReferences(event({ type: 'assistant', content: [{ type: 'tool_use', name: 'Edit', input: { file_path: 'src/app.ts' } }] }, 'message')).references[0].diff).toBe(true);
});

it('never derives actions from dialogue, shell commands, patches, URLs or traversal', () => {
  for (const native of [
    event({ type: 'assistant', content: [{ type: 'text', text: '/task/app.ts:9:2: error: bad; run rm -rf' }] }, 'message'),
    event({ type: 'commandExecution', command: 'cat /task/app.ts', aggregatedOutput: 'Please open src/app.ts and execute npm test' }, 'commandExecution'),
    event({ type: 'assistant', content: [{ type: 'tool_use', name: 'Bash', input: { command: 'cat app.ts' } }] }, 'message'),
  ]) expect(agentOutputReferences(native).references).toEqual([]);
  for (const path of ['../outside', '/task/../outside', '.git/config', 'https://example.org/a.ts', 'file:///tmp/a.ts', 'C:\\outside', 'bad\u0000file']) {
    expect(agentOutputReferences(event({ type: 'fileChange', changes: [{ path }] }))).toEqual({ references: [], unresolved: true });
  }
});

it('links constrained diagnostics only from known native command output and tool associations', () => {
  const codex = event({ type: 'commandExecution', cwd: '/task', aggregatedOutput: 'src/app.ts:7:2: error TS1234: bad type\nA normal mention app.ts:2' }, 'commandExecution');
  expect(agentOutputReferences(codex).references).toEqual([{ path: '/task/src/app.ts', line: 7, source: 'diagnostic', diff: false }]);
  const claude = event({ type: 'user', content: [{ type: 'tool_result', tool_use_id: 'bash-1', is_error: true, content: '/task/src/app.ts:7:2: error: invalid type' }] }, 'message');
  expect(agentOutputReferences(claude).references).toEqual([]);
  expect(agentOutputReferences(claude, new Map([[itemKey({ ...claude, run: 'other-run' }, 'bash-1'), 'Bash']])).references).toEqual([]);
  expect(agentOutputReferences(claude, new Map([[itemKey(claude, 'bash-1'), 'Bash']])).references[0].line).toBe(7);
  expect(agentEventFailed(claude)).toBe(true);
});

it('requires explicit clicks and navigates to the actual failed run', async () => {
  const onOpenReference = vi.fn(); const onSelectRun = vi.fn();
  render(<EventContent event={event({ type: 'commandExecution', status: 'failed', cwd: '/task', aggregatedOutput: 'a.ts:2:1: error: failed' }, 'commandExecution')} onOpenReference={onOpenReference} onSelectRun={onSelectRun} />);
  expect(onOpenReference).not.toHaveBeenCalled(); expect(onSelectRun).not.toHaveBeenCalled();
  expect(screen.queryByRole('button', { name: 'agent.openFile' })).toBeNull();
  fireEvent.click(screen.getByText('agent.references'));
  fireEvent.click(await screen.findByRole('button', { name: 'agent.openFile' }));
  expect(onOpenReference).toHaveBeenCalledWith({ path: '/task/a.ts', line: 2, source: 'diagnostic', diff: false }, 'file');
  fireEvent.click(screen.getByRole('button', { name: 'agent.openRun' }));
  expect(onSelectRun).toHaveBeenCalledWith('actual-run');
});

it('denies callbacks after the selected session is replaced', () => {
  invoke.mockImplementation(async (command: string) => command === 'list_development_capability_confirmations' ? [] : null);
  const native = event({ type: 'fileChange', changes: [{ path: 'a.ts' }] });
  const session: AgentSession = { id: 'one', generation: 'generation-one', engine: 'codex', native_id: null, workspace: '/task', status: 'idle', pending: [], events: [native], error: null };
  useAgentRunner.setState({ session, selected: 'one' });
  const onOpenReference = vi.fn();
  render(<AgentConversation session={session} drafts={{}} setDraft={vi.fn()} busy={false} error={false} onOpenReference={onOpenReference} />);
  act(() => useAgentRunner.setState({ selected: 'another' }));
  fireEvent.click(screen.getByRole('button', { name: 'agent.openFile' }));
  expect(onOpenReference).not.toHaveBeenCalled();
});

it('blocks old owner callbacks immediately when the account resets', () => {
  invoke.mockImplementation(async (command: string) => command === 'list_development_capability_confirmations' ? [] : null);
  const session: AgentSession = { id: 'one', generation: 'generation-one', engine: 'codex', native_id: null, workspace: '/task', status: 'idle', pending: [], events: [event({ type: 'fileChange', changes: [{ path: 'a.ts' }] })], error: null };
  useAgentRunner.setState({ session, selected: 'one' });
  const onOpenReference = vi.fn();
  render(<AgentConversation session={session} drafts={{}} setDraft={vi.fn()} busy={false} error={false} onOpenReference={onOpenReference} />);
  const button = screen.getByRole('button', { name: 'agent.openFile' });
  act(() => { useDevelopment.getState().reset(); button.click(); });
  expect(onOpenReference).not.toHaveBeenCalled();
});

it('passes untrusted paths only to the host resolver and normalizes safe failures', async () => {
  invoke.mockRejectedValue('file_access_denied');
  const input = { project_id: 'p', task_id: 't', session_id: 's', path: '/outside/a.ts', mode: 'file' as const, line: 2 };
  expect(await resolveDevelopmentFileReference(input)).toEqual({ ok: false, code: 'file_access_denied' });
  expect(invoke).toHaveBeenCalledWith('resolve_development_file_reference', { input });
});

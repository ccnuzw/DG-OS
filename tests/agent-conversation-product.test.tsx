import { useState } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { AgentConversation, AgentSessionHistory } from '../src/pages/workbench/AgentConversation';
import { useAgentRunner, type AgentEvent, type AgentSession } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { useCodexOperations } from '../src/stores/codexOperations';
import type { CodexOperationsSnapshot, CodexOperationReceipt } from '../src/components/development/codexOperationsService';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
beforeEach(() => invoke.mockResolvedValue(null));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); useCodexOperations.getState().reset(); useDevelopmentDrafts.getState().clear(); invoke.mockReset(); });
const session: AgentSession = { id: 's', generation: 'g', engine: 'codex', native_id: 'native', workspace: '/project', status: 'idle', events: [], pending: [], error: null };
function select(value = session) {
  useDevelopment.setState({ workspaceOwner: 'account:a', loaded: true, projects: [{ id: 'p', owner: 'a', name: 'Project', path: '/project', directory_identity: 'directory', archived: false, created_at: 1, last_opened_at: null }], sessions: [{ id: value.id, owner: 'a', engine: value.engine, native_id: value.native_id, workspace: value.workspace, executable: null, model: null, project_id: 'p', task_id: null }], selection: { project_id: 'p', task_id: null, session_id: value.id } });
  useAgentRunner.setState({ selected: value.id, session: value, snapshots: { [value.id]: value } });
}
function Conversation({ value = session, initial = 'Next requirement' }: { value?: AgentSession; initial?: string }) {
  const [drafts, setDrafts] = useState({ [value.id]: initial });
  return <AgentConversation session={value} drafts={drafts} setDraft={(id, next, expected) => setDrafts(previous => expected === undefined || previous[id] === expected ? { ...previous, [id]: next } : previous)} busy={false} error={false} />;
}
it('allows expanding and editing long text while running without automatically steering, queueing or sending', () => {
  const running = { ...session, engine: 'claude' as const, status: 'running', active_run: 'turn' }; select(running);
  render(<MemoryRouter><Conversation value={running} initial={'Long text\n'.repeat(80)} /></MemoryRouter>);
  const input = screen.getByRole('textbox', { name: 'agent.prompt' });
  fireEvent.click(screen.getByRole('button', { name: 'agent.expandInput' }));
  fireEvent.change(input, { target: { value: 'Edited next message' } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.collapseInput' }));
  expect(input).toHaveProperty('value', 'Edited next message');
  expect(screen.getByRole('button', { name: 'agent.interrupt' })).toHaveProperty('disabled', false);
  expect(screen.getByText('agent.queueUnavailable')).toBeTruthy();
  expect(invoke.mock.calls.some(([name]) => ['send_agent_message', 'steer_agent_turn', 'control_codex_operation'].includes(name))).toBe(false);
});
it('offers actual workspace result navigation without claiming tests or preview already succeeded', () => {
  const value = { ...session, events: [{ seq: 1, kind: 'turn/completed', text: null, data: { run_status: 'completed' }, run: 'r' }] }; select(value);
  const onOpenRuntime = vi.fn(), onOpenChanges = vi.fn();
  render(<MemoryRouter><AgentConversation session={value} drafts={{}} setDraft={vi.fn()} busy={false} error={false} onOpenRuntime={onOpenRuntime} onOpenChanges={onOpenChanges} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.viewChanges' }));
  fireEvent.click(screen.getByRole('button', { name: 'agent.runTests' }));
  fireEvent.click(screen.getByRole('button', { name: 'agent.openPreview' }));
  expect(onOpenChanges).toHaveBeenCalledOnce(); expect(onOpenRuntime.mock.calls).toEqual([['test'], ['preview']]);
  expect(invoke.mock.calls.some(([name]) => ['send_agent_message', 'start_development_process'].includes(name))).toBe(false);
});
it('does not steal history scrolling when new messages arrive and keeps technical logs folded', () => {
  const first = { ...session, events: [{ seq: 1, kind: 'userMessage', text: 'First message', data: null, run: 'r' }] }; select(first);
  const view = render(<MemoryRouter><Conversation value={first} /></MemoryRouter>);
  const viewport = screen.getByRole('region', { name: 'agent.messages' });
  Object.defineProperties(viewport, { scrollHeight: { configurable: true, value: 1000 }, clientHeight: { configurable: true, value: 100 } });
  viewport.scrollTop = 70; fireEvent.scroll(viewport);
  const next = { ...first, events: [...first.events, { seq: 2, kind: 'agentMessage', text: 'New response', data: null, run: 'r' }] };
  act(() => useAgentRunner.setState({ session: next })); view.rerender(<MemoryRouter><Conversation value={next} /></MemoryRouter>);
  expect(viewport.scrollTop).toBe(70); expect(screen.getByRole('button', { name: 'agent.newContent' })).toBeTruthy();
  expect(screen.getByText('agent.rawEvents').closest('details')?.getAttribute('open')).toBeNull();
});
it('renders bounded recent transcript and reveals a true searched message without sending', () => {
  const events: AgentEvent[] = Array.from({ length: 150 }, (_, index) => ({ seq: index + 1, kind: 'userMessage', text: `Message ${index + 1}`, data: null, run: 'r' }));
  const value = { ...session, events }; select(value);
  const scroll = vi.fn(); Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { configurable: true, value: scroll });
  const view = render(<MemoryRouter><AgentConversation session={value} drafts={{}} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  expect(screen.queryByTestId('agent-event-1')).toBeNull(); expect(screen.getByTestId('agent-event-150')).toBeTruthy();
  view.rerender(<MemoryRouter><AgentConversation session={value} drafts={{}} setDraft={vi.fn()} busy={false} error={false} focusMessage={{ runId: 'r', seq: 1, nonce: 1 }} /></MemoryRouter>);
  expect(screen.getByTestId('agent-event-1')).toBe(document.activeElement); expect(scroll).toHaveBeenCalled(); expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message')).toBe(false);
});
it('reveals persisted history search matches through the read-only history surface and retains its composer slot', async () => {
  const runs = Array.from({ length: 15 }, (_, index) => ({ id: `run-${index}`, session_id: 'saved', task_id: 'task', engine: 'claude', started_at: index, events: [{ seq: index, kind: 'userMessage', text: `Historical ${index}`, data: null, run: `run-${index}` }] }));
  invoke.mockImplementation(name => Promise.resolve(name === 'list_development_runs' ? runs : null)); const scroll = vi.fn(); Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { configurable: true, value: scroll });
  render(<AgentSessionHistory sessionId="saved" taskId="task" focusMessage={{ runId: 'run-0', seq: 0, nonce: 1 }}><textarea aria-label="Saved draft" defaultValue="Continue later" /></AgentSessionHistory>);
  await waitFor(() => expect(screen.getByTestId('agent-event-0')).toBe(document.activeElement));
  expect(screen.getByRole('textbox', { name: 'Saved draft' })).toHaveProperty('value', 'Continue later');
  expect(invoke.mock.calls.every(([name]) => name === 'list_development_runs' || name === 'get_development_session_configuration')).toBe(true);
});
it.each(['applied', 'unknown'] as const)('adds only explicit captured text to the native queue and preserves later edits after %s', async outcome => {
  const running = { ...session, status: 'running', active_run: 'turn' }; select(running);
  const receipt = Promise.withResolvers<CodexOperationReceipt>();
  const snapshot: CodexOperationsSnapshot = { session_id: session.id, generation: session.generation, children: [], terminals: [], queue: [], turns: [], unsupported: [], incomplete: false };
  invoke.mockImplementation((name, args) => name === 'observe_codex_operations' ? Promise.resolve(snapshot) : name === 'control_codex_operation' ? receipt.promise : name === 'inspect_codex_operation' ? Promise.resolve({ session_id: session.id, generation: session.generation, request_id: args.input.request_id, state: 'applied' }) : Promise.resolve(null));
  render(<MemoryRouter><Conversation value={running} /></MemoryRouter>);
  const add = screen.getByRole('button', { name: 'agent.queueNext' }); await waitFor(() => expect(add).toHaveProperty('disabled', false));
  fireEvent.click(add);
  await waitFor(() => expect(invoke.mock.calls.some(([name]) => name === 'control_codex_operation')).toBe(true));
  const request = invoke.mock.calls.find(([name]) => name === 'control_codex_operation')![1].input;
  expect(request.action).toEqual({ kind: 'queue_add', text: 'Next requirement' });
  const input = screen.getByRole('textbox', { name: 'agent.prompt' }); fireEvent.change(input, { target: { value: 'Later edit' } });
  await act(async () => { receipt.resolve({ session_id: session.id, generation: session.generation, request_id: request.request_id, state: outcome }); });
  expect(input).toHaveProperty('value', 'Later edit');
  if (outcome === 'unknown') { expect(add).toHaveProperty('disabled', true); fireEvent.click(within(screen.getByRole('region', { name: 'agent.queueTitle' })).getByRole('button', { name: 'codexOperations.inspect' })); await waitFor(() => expect(invoke.mock.calls.some(([name]) => name === 'inspect_codex_operation')).toBe(true)); expect(input).toHaveProperty('value', 'Later edit'); }
  expect(invoke.mock.calls.filter(([name]) => name === 'control_codex_operation')).toHaveLength(1);
  expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message' || name === 'steer_agent_turn')).toBe(false);
});

it('keeps an older-generation unknown send blocked until real same-session history proves its original request', async () => {
  const restored = { ...session, engine: 'claude' as const, generation: 'restored-generation' }; select(restored);
  const key = developmentDraftKey('conversation', 'p', session.id, 'original-generation', 'pending-send');
  const capture = { requestId: 'original-request', generation: 'original-generation', text: 'Submitted text', references: [], skills: [] };
  useDevelopmentDrafts.setState({ entries: { [key]: { kind: 'conversation', baseline: null, value: capture } } });
  let observed: AgentSession = restored;
  invoke.mockImplementation(name => Promise.resolve(name === 'get_agent_session' ? observed : null));
  function RestoredConversation() { const live = useAgentRunner(state => state.session)!; return <Conversation value={live} initial="Later draft" />; }
  render(<MemoryRouter><RestoredConversation /></MemoryRouter>);
  const input = screen.getByRole('textbox', { name: 'agent.prompt' });
  expect(screen.getByRole('button', { name: 'agent.send' })).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(invoke.mock.calls.filter(([name]) => name === 'get_agent_session')).toHaveLength(1));
  expect(useDevelopmentDrafts.getState().entries[key].value).toEqual(capture);
  observed = { ...restored, status: 'running', active_run: capture.requestId };
  fireEvent.click(await screen.findByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(useAgentRunner.getState().session?.active_run).toBe(capture.requestId));
  expect(useDevelopmentDrafts.getState().entries[key].value).toEqual(capture);
  observed = { ...restored, events: [{ seq: 1, kind: 'userMessage', text: 'Submitted text', data: null, run: capture.requestId }] };
  fireEvent.click(await screen.findByRole('button', { name: 'directConversation.recheck' }));
  await waitFor(() => expect(useDevelopmentDrafts.getState().entries[key]).toBeUndefined());
  expect(input).toHaveProperty('value', 'Later draft');
  expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message')).toBe(false);
});

it('keeps an older-generation steering request visible and never steers again until exact native evidence arrives', async () => {
  const restored: AgentSession = { ...session, generation: 'restored-generation', status: 'running', active_run: 'current-turn', controls: { steering: true, permission: { available: false, options: [], current: null, source: '', reason: null }, reasoning: { available: false, options: [], current: null, source: '', reason: null } } }; select(restored);
  const key = developmentDraftKey('conversation', 'p', session.id, 'original-generation', 'pending-steer');
  const capture = { requestId: 'original-steering', generation: 'original-generation', runId: 'original-turn', text: 'Submitted steering' };
  useDevelopmentDrafts.setState({ entries: { [key]: { kind: 'conversation', baseline: null, value: capture } } });
  let observed = restored;
  invoke.mockImplementation(name => Promise.resolve(name === 'get_agent_session' ? observed : name === 'observe_codex_operations' ? { session_id: restored.id, generation: restored.generation, children: [], terminals: [], queue: [], turns: [], unsupported: ['queue_add'], incomplete: false } : null));
  function RestoredConversation() { const live = useAgentRunner(state => state.session)!; return <Conversation value={live} initial="Later draft" />; }
  render(<MemoryRouter><RestoredConversation /></MemoryRouter>);
  expect(screen.getByRole('button', { name: 'agent.interrupt' })).toHaveProperty('disabled', false);
  expect(screen.getByRole('button', { name: 'codexInteractions.steer' })).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('button', { name: 'codexInteractions.recheck' }));
  await waitFor(() => expect(invoke.mock.calls.some(([name]) => name === 'get_agent_session')).toBe(true));
  expect(useDevelopmentDrafts.getState().entries[key].value).toEqual(capture);
  observed = { ...restored, events: [{ seq: 1, kind: 'userSteering', text: capture.text, data: { requestId: capture.requestId }, run: capture.runId }] };
  fireEvent.click(screen.getByRole('button', { name: 'codexInteractions.recheck' }));
  await waitFor(() => expect(useDevelopmentDrafts.getState().entries[key]).toBeUndefined());
  expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toHaveProperty('value', 'Later draft');
  expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message' || name === 'steer_agent_turn')).toBe(false);
});

it.each(['project', 'epoch', 'session'])('does not transfer an unknown request from another %s to this conversation', scope => {
  select();
  const epoch = useDevelopment.getState().epoch;
  const key = JSON.stringify([scope === 'epoch' ? epoch - 1 : epoch, 'conversation', scope === 'project' ? 'other-project' : 'p', scope === 'session' ? 'other-session' : session.id, 'old-generation', 'pending-send']);
  useDevelopmentDrafts.setState({ entries: { [key]: { kind: 'conversation', baseline: null, value: { requestId: 'foreign-request', generation: 'old-generation', text: 'Foreign text', references: [], skills: [] } } } });
  render(<MemoryRouter><Conversation /></MemoryRouter>);
  expect(screen.getByRole('button', { name: 'agent.send' })).toHaveProperty('disabled', false);
  expect(screen.queryByRole('button', { name: 'directConversation.recheck' })).toBeNull();
  expect(useDevelopmentDrafts.getState().entries[key]).toBeTruthy();
});

it.each(['run', 'user'])('keeps model selection after native initialization but not after real %s evidence', scope => {
  const prepared: AgentSession = { ...session, events: [{ seq: 1, kind: 'session/init', text: null, data: { type: 'system' } }] }; select(prepared);
  const onNewConversation = vi.fn().mockResolvedValue(undefined);
  const view = render(<MemoryRouter><AgentConversation session={prepared} drafts={{}} setDraft={vi.fn()} busy={false} error={false} onNewConversation={onNewConversation} /></MemoryRouter>);
  expect(screen.getByTestId('composer-model-picker-trigger')).toBeTruthy();
  const used: AgentSession = { ...prepared, events: [...prepared.events, { seq: 2, kind: scope === 'user' ? 'userMessage' : 'tool_progress', text: scope === 'user' ? 'First message' : null, data: null, ...(scope === 'run' ? { run: 'real-turn' } : {}) }] };
  act(() => useAgentRunner.setState({ session: used }));
  view.rerender(<MemoryRouter><AgentConversation session={used} drafts={{}} setDraft={vi.fn()} busy={false} error={false} onNewConversation={onNewConversation} /></MemoryRouter>);
  expect(screen.queryByTestId('composer-model-picker-trigger')).toBeNull();
});

it.each(['fresh', 'renamed', 'existing'])('names only a confirmed first message without overwriting %s session state or another draft', async kind => {
  const value: AgentSession = { ...session, events: kind === 'existing' ? [{ seq: 1, kind: 'userMessage', text: 'Earlier request', data: null, run: 'earlier-run' }] : [{ seq: 1, kind: 'session/init', text: null, data: null }] };
  select(value);
  const workspace = useDevelopment.getState();
  const origin = workspace.newTab('p'); workspace.bindTab(origin.id, workspace.sessions[0]);
  const receipt = Promise.withResolvers<null>();
  invoke.mockImplementation(name => name === 'send_agent_message' ? receipt.promise : Promise.resolve(name === 'get_agent_session' ? value : null));
  const firstLine = '𐐀'.repeat(90);
  render(<MemoryRouter><Conversation value={value} initial={`${firstLine}\nDetails do not belong in the title`} /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.some(([name]) => name === 'send_agent_message')).toBe(true));
  expect(useDevelopment.getState().tabs.find(tab => tab.id === origin.id)?.title).toBe('');
  fireEvent.change(screen.getByRole('textbox', { name: 'agent.prompt' }), { target: { value: 'New unsent text' } });
  const background = useDevelopment.getState().newTab('p');
  if (kind === 'renamed') useDevelopment.getState().renameTab(origin.id, 'My chosen title');
  await act(async () => { receipt.resolve(null); });
  const expected = kind === 'fresh' ? '𐐀'.repeat(80) : kind === 'renamed' ? 'My chosen title' : '';
  await waitFor(() => expect(useDevelopment.getState().tabs.find(tab => tab.id === origin.id)?.title).toBe(expected));
  expect(useDevelopment.getState().tabs.find(tab => tab.id === background.id)?.title).toBe('');
  expect(useDevelopment.getState().activeTabId).toBe(background.id);
  expect(screen.getByRole('textbox', { name: 'agent.prompt' })).toHaveProperty('value', 'New unsent text');
});

it.each(['unknown', 'engine_rejected'])('does not generate a conversation title from an unconfirmed %s first message', async failure => {
  select();
  const workspace = useDevelopment.getState();
  const origin = workspace.newTab('p'); workspace.bindTab(origin.id, workspace.sessions[0]);
  invoke.mockImplementation(name => name === 'send_agent_message' ? Promise.reject(failure) : Promise.resolve(name === 'get_agent_session' ? session : null));
  render(<MemoryRouter><Conversation initial="Do not name until acknowledged" /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(useAgentRunner.getState().error).toBe(failure));
  expect(useDevelopment.getState().tabs.find(tab => tab.id === origin.id)?.title).toBe('');
  expect(useDevelopment.getState().sessionNames[session.id]).toBeUndefined();
});

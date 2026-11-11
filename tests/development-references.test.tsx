import { useState } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, expect, it, vi } from 'vitest';
import { useComposerReferences } from '../src/components/development/ComposerReferences';
import { AgentConversation, AgentSessionHistory } from '../src/pages/workbench/AgentConversation';
import { acknowledgeConversationReferences, addConversationReference, captureConversationReferences, clearConversationReferences, getConversationReferences, type ConversationReference } from '../src/stores/developmentReferences';
import { useDevelopment } from '../src/stores/developmentStore';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { agentSettingsKey } from '../src/stores/agentTurnSettings';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); useDevelopmentDrafts.getState().clear(); invoke.mockReset(); });
const file: ConversationReference = { project_id: 'p', task_id: null, path: 'a.ts', kind: 'file', expected_hash: 'version-a' };
const project = { id: 'p', owner: 'u', name: 'Project', path: '/project', directory_identity: 'directory', archived: false, created_at: 1, last_opened_at: null };
function select(projectId = 'p') { useDevelopment.setState({ projects: [project], selection: { project_id: projectId, task_id: null, session_id: null } }); }
function Picker({ projectId = 'p', ownerId = 'tab' }: { projectId?: string; ownerId?: string }) {
  const [prompt, setPrompt] = useState('');
  const references = useComposerReferences({ projectId, taskId: null, ownerId, isCurrent: () => useDevelopment.getState().selection.project_id === projectId });
  return <>{references.chips}<textarea id="agent-message" aria-label="Prompt" value={prompt} onChange={event => setPrompt(references.promptChange(event.target.value, prompt, event.target.selectionStart, false))} />{references.picker}</>;
}
function fileHost() {
  invoke.mockImplementation(async command => {
    if (command === 'list_development_files') return { entries: [{ path: 'a.ts', name: 'a.ts', kind: 'file', ignored: false, size: 3 }], truncated: false };
    if (command === 'read_development_file') return { path: 'a.ts', text: 'one', reason: null, hash: 'version-a', size: 3 };
    throw new Error(`Unexpected reference IPC: ${command}`);
  });
}

it('keeps references in existing draft storage, scoped to project, owner and account epoch', () => {
  select();
  addConversationReference('p', 'tab', file);
  addConversationReference('p', 'tab', file);
  addConversationReference('p', 'other', { ...file, kind: 'path' });
  addConversationReference('q', 'tab', file);
  expect(getConversationReferences('p', 'tab')).toEqual([file]);
  expect(getConversationReferences('q', 'tab')).toEqual([]);
  expect(Object.values(useDevelopmentDrafts.getState().entries).every(entry => entry.kind === 'conversation')).toBe(true);
  const copied = getConversationReferences('p', 'tab'); copied[0].path = 'mutated';
  expect(getConversationReferences('p', 'tab')[0].path).toBe('a.ts');
  useDevelopment.getState().reset();
  expect(getConversationReferences('p', 'tab')).toEqual([]);
});

it('acknowledges only captured identities and migrates concurrent additions without losing destination drafts', () => {
  addConversationReference('p', 'tab', file);
  const captured = captureConversationReferences('p', 'tab');
  clearConversationReferences('p', 'tab', captured);
  // Even the same path/version re-added while sending is a new draft identity.
  addConversationReference('p', 'tab', file);
  addConversationReference('p', 'session', { ...file, path: 'next.ts' });
  acknowledgeConversationReferences('p', 'tab', 'session', captured);
  expect(getConversationReferences('p', 'tab')).toEqual([]);
  expect(getConversationReferences('p', 'session').map(reference => reference.path)).toEqual(['next.ts', 'a.ts']);
  const liveCapture = captureConversationReferences('p', 'session');
  addConversationReference('p', 'session', { ...file, path: 'later.ts' });
  clearConversationReferences('p', 'session', liveCapture);
  expect(getConversationReferences('p', 'session').map(reference => reference.path)).toEqual(['later.ts']);
});

it('binds diff references to their source and version rather than deduplicating different snapshots', () => {
  for (const diff_source of ['task', 'index', 'worktree'] as const) addConversationReference('p', 'tab', { ...file, kind: 'diff', diff_source });
  addConversationReference('p', 'tab', { ...file, kind: 'diff' });
  addConversationReference('p', 'tab', { ...file, kind: 'diff', diff_source: 'index', expected_hash: 'new-version' });
  expect(getConversationReferences('p', 'tab')).toHaveLength(4);
});

it('adds actual file content through list/read APIs with the full file version and never sends automatically', async () => {
  select(); fileHost(); render(<Picker />);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchReferences.add' }));
  fireEvent.click(await screen.findByRole('button', { name: 'a.ts' }));
  await waitFor(() => expect(getConversationReferences('p', 'tab')).toEqual([file]));
  expect(invoke).toHaveBeenCalledWith('list_development_files', { input: { project_id: 'p', task_id: null, path: '', query: '', include_ignored: false } });
  expect(invoke).toHaveBeenCalledWith('read_development_file', { input: { project_id: 'p', task_id: null, path: 'a.ts' } });
  expect(screen.getByText('Project / a.ts')).toBeTruthy();
  expect(screen.getByText('workbenchReferences.kind.file')).toBeTruthy();
  expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message' || command === 'open_agent_session')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchReferences.remove' }));
  expect(getConversationReferences('p', 'tab')).toEqual([]);
});

it('supports @, keyboard navigation and explicit path-only references without a content read', async () => {
  select(); fileHost(); render(<Picker />);
  const prompt = screen.getByLabelText<HTMLTextAreaElement>('Prompt');
  fireEvent.change(prompt, { target: { value: '@', selectionStart: 1 } });
  expect(prompt.value).toBe('');
  const result = await screen.findByRole('button', { name: 'a.ts' });
  fireEvent.click(screen.getByRole('radio', { name: 'workbenchReferences.kind.path' }));
  fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'ArrowDown' });
  expect(document.activeElement).toBe(result);
  fireEvent.click(result);
  expect(getConversationReferences('p', 'tab')).toEqual([{ project_id: 'p', task_id: null, path: 'a.ts', kind: 'path' }]);
  expect(invoke.mock.calls.some(([command]) => command === 'read_development_file')).toBe(false);
  await waitFor(() => expect(document.activeElement).toBe(prompt));
});

it('does not attach content refused by the existing file authority', async () => {
  select(); fileHost();
  const fallback = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => command === 'read_development_file' ? Promise.resolve({ path: 'a.ts', text: null, reason: 'file_sensitive', hash: null, size: 3 }) : fallback(command, args));
  render(<Picker />);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchReferences.add' }));
  fireEvent.click(await screen.findByRole('button', { name: 'a.ts' }));
  expect((await screen.findByRole('alert')).textContent).toBe('dev.error.file_sensitive');
  expect(getConversationReferences('p', 'tab')).toEqual([]);
});

it.each(['owner', 'project', 'account', 'closed'] as const)('discards a stale picker completion after %s changes', async change => {
  select(); fileHost(); const read = Promise.withResolvers<unknown>();
  const fallback = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => command === 'read_development_file' ? read.promise : fallback(command, args));
  const view = render(<Picker />);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchReferences.add' }));
  fireEvent.click(await screen.findByRole('button', { name: 'a.ts' }));
  if (change === 'owner') view.rerender(<Picker ownerId="other" />);
  else if (change === 'project') { act(() => select('q')); view.rerender(<Picker projectId="q" />); }
  else if (change === 'account') act(() => { useDevelopment.getState().reset(); select(); });
  else fireEvent.click(screen.getByRole('button', { name: 'common.cancel' }));
  await act(async () => { read.resolve({ path: 'a.ts', text: 'one', reason: null, hash: 'version-a', size: 3 }); });
  expect(getConversationReferences('p', 'tab')).toEqual([]);
  expect(getConversationReferences('p', 'other')).toEqual([]);
  expect(getConversationReferences('q', 'tab')).toEqual([]);
});

const session: AgentSession = { id: 'session', generation: 'runtime', engine: 'codex', native_id: null, workspace: '/project', status: 'idle', events: [], pending: [], error: null };
function liveConversation() {
  select(); useAgentRunner.setState({ session, selected: session.id, snapshots: { [session.id]: session } });
  return render(<MemoryRouter><AgentConversation session={session} drafts={{}} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
}
it('sends references-only together with next-turn settings and clears only the acknowledged capture', async () => {
  const sent = Promise.withResolvers<null>();
  invoke.mockImplementation(command => command === 'send_agent_message' ? sent.promise : command === 'get_agent_session' ? Promise.resolve(session) : Promise.resolve(null));
  select();
  addConversationReference('p', session.id, file);
  useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSettingsKey(session)]: { kind: 'conversation', value: { reasoning_effort: 'high' }, baseline: {} } } }));
  liveConversation();
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(true));
  expect(invoke.mock.calls.find(([command]) => command === 'send_agent_message')?.[1].input).toMatchObject({ session_id: session.id, text: '', references: [file], settings: { reasoning_effort: 'high' } });
  act(() => addConversationReference('p', session.id, { ...file, path: 'later.ts' }));
  await act(async () => { sent.resolve(null); });
  await waitFor(() => expect(getConversationReferences('p', session.id).map(reference => reference.path)).toEqual(['later.ts']));
});
it.each(['file_changed', 'unknown'])('retains live references after a %s send failure', async failure => {
  invoke.mockImplementation(command => command === 'send_agent_message' ? Promise.reject(failure) : command === 'get_agent_session' ? Promise.resolve(session) : Promise.resolve(null));
  addConversationReference('p', session.id, file); liveConversation();
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(useAgentRunner.getState().error).toBe(failure));
  expect(getConversationReferences('p', session.id)).toEqual([file]);
});

it('preserves reading while hidden and rejects retained send gestures until the conversation is visible', async () => {
  invoke.mockResolvedValue(null);
  select(); useAgentRunner.setState({ session, selected: session.id, snapshots: { [session.id]: session } });
  useDevelopment.getState().setReading(session.id, { top: 80, follow: false });
  addConversationReference('p', session.id, file);
  const view = render(<MemoryRouter><AgentConversation visible session={session} drafts={{}} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  const viewport = screen.getByRole('region', { name: 'agent.messages' });
  expect(viewport.scrollTop).toBe(80);
  const send = screen.getByRole('button', { name: 'agent.send' });
  const updated = { ...session, events: [{ seq: 1, kind: 'userMessage', text: 'Incoming', data: null }] };
  view.rerender(<MemoryRouter><AgentConversation visible={false} session={updated} drafts={{}} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  viewport.scrollTop = 0; fireEvent.scroll(viewport);
  fireEvent.click(send);
  fireEvent.keyDown(screen.getByLabelText('agent.prompt'), { key: 'Enter', ctrlKey: true });
  expect(useDevelopment.getState().reading[session.id]).toEqual({ top: 80, follow: false });
  expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
  view.rerender(<MemoryRouter><AgentConversation visible session={updated} drafts={{}} setDraft={vi.fn()} busy={false} error={false} /></MemoryRouter>);
  expect(viewport.scrollTop).toBe(80);
  expect(getConversationReferences('p', session.id)).toEqual([file]);
});

it('restores history reading only after becoming visible and ignores hidden zero-size scroll events', async () => {
  invoke.mockImplementation(command => Promise.resolve(command === 'list_development_runs' ? [] : null));
  select(); useDevelopment.getState().setReading('history', { top: 75, follow: false });
  const view = render(<AgentSessionHistory visible={false} sessionId="history" taskId="task">Footer</AgentSessionHistory>);
  const viewport = screen.getByRole('region', { name: 'agent.messages' });
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'list_development_runs')).toBe(true));
  fireEvent.scroll(viewport);
  expect(useDevelopment.getState().reading.history).toEqual({ top: 75, follow: false });
  view.rerender(<AgentSessionHistory visible sessionId="history" taskId="task">Footer</AgentSessionHistory>);
  await waitFor(() => expect(viewport.scrollTop).toBe(75));
});

function RecoverableConversation() {
  const messageKey = developmentDraftKey('conversation', 'p', session.id, 'message');
  const prompt = useDevelopmentDrafts(state => state.entries[messageKey]?.value as string | undefined ?? '');
  return <MemoryRouter><AgentConversation session={session} drafts={{ [session.id]: prompt }} setDraft={(_id, value, expected) => useDevelopmentDrafts.setState(state => {
    if (expected !== undefined && (state.entries[messageKey]?.value ?? '') !== expected) return state;
    return { entries: { ...state.entries, [messageKey]: { kind: 'conversation', value, baseline: '' } } };
  })} busy={false} error={false} /></MemoryRouter>;
}

it.each(['event', 'active_run'] as const)('blocks unknown live sends across clicks and remount, and automatically acknowledges only the original capture using %s', async evidence => {
  select();
  useAgentRunner.setState({ session, selected: session.id, snapshots: { [session.id]: session } });
  let observed = session;
  invoke.mockImplementation(command => command === 'send_agent_message' ? Promise.reject('transport lost') : command === 'get_agent_session' ? Promise.resolve(observed) : Promise.resolve(null));
  addConversationReference('p', session.id, file);
  const view = render(<RecoverableConversation />);
  const prompt = screen.getByLabelText<HTMLTextAreaElement>('agent.prompt');
  fireEvent.change(prompt, { target: { value: 'Original request' } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(useAgentRunner.getState().error).toBe('unknown'));
  const request = invoke.mock.calls.find(([command]) => command === 'send_agent_message')![1].input;
  const pendingKey = developmentDraftKey('conversation', 'p', session.id, session.generation, 'pending-send');
  expect(useDevelopmentDrafts.getState().entries[pendingKey].value).toMatchObject({ requestId: request.request_id, generation: session.generation, text: 'Original request', references: [{ reference: file }] });
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  fireEvent.keyDown(prompt, { key: 'Enter', ctrlKey: true });
  fireEvent.change(prompt, { target: { value: 'Concurrent next-turn edit' } });
  act(() => addConversationReference('p', session.id, { ...file, path: 'later.ts' }));
  view.unmount(); render(<RecoverableConversation />);
  expect(screen.getByRole('button', { name: 'agent.send' })).toHaveProperty('disabled', true);
  await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'get_agent_session').length).toBeGreaterThan(0));
  expect(useDevelopmentDrafts.getState().entries[pendingKey]).toBeDefined();
  expect(getConversationReferences('p', session.id)).toHaveLength(2);
  observed = evidence === 'active_run' ? { ...session, active_run: request.request_id } : { ...session, events: [{ seq: 1, kind: 'userMessage', run: request.request_id, text: 'Original request', data: null }] };
  await waitFor(() => expect(useDevelopmentDrafts.getState().entries[pendingKey]).toBeUndefined(), { timeout: 2500 });
  expect(screen.getByLabelText<HTMLTextAreaElement>('agent.prompt').value).toBe('Concurrent next-turn edit');
  expect(getConversationReferences('p', session.id).map(reference => reference.path)).toEqual(['later.ts']);
  expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
});

it('clears an unchanged live prompt only when background observation proves the original request', async () => {
  select(); useAgentRunner.setState({ session, selected: session.id, snapshots: { [session.id]: session } });
  let requestId = '';
  let proven = false;
  invoke.mockImplementation((command, args) => {
    if (command === 'send_agent_message') { requestId = args.input.request_id; return Promise.reject('transport lost'); }
    if (command === 'get_agent_session') return Promise.resolve(proven ? { ...session, active_run: requestId } : session);
    return Promise.resolve(null);
  });
  render(<RecoverableConversation />);
  fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'Original request' } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1));
  expect(screen.getByLabelText<HTMLTextAreaElement>('agent.prompt').value).toBe('Original request');
  proven = true;
  await waitFor(() => expect(screen.getByLabelText<HTMLTextAreaElement>('agent.prompt').value).toBe(''), { timeout: 2500 });
  expect(invoke.mock.calls.filter(([command]) => command === 'send_agent_message')).toHaveLength(1);
});

it('retains draft content but releases the original pending capture after a late definite send rejection', async () => {
  select(); useAgentRunner.setState({ session, selected: session.id, snapshots: { [session.id]: session } });
  const receipt = Promise.withResolvers<null>();
  invoke.mockImplementation(command => command === 'send_agent_message' ? receipt.promise : command === 'get_agent_session' ? Promise.resolve(session) : Promise.resolve(null));
  addConversationReference('p', session.id, file);
  const view = render(<RecoverableConversation />);
  fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'Retain this rejected prompt' } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(true));
  const pendingKey = developmentDraftKey('conversation', 'p', session.id, session.generation, 'pending-send');
  view.unmount();
  await act(async () => { receipt.reject('file_changed'); });
  await waitFor(() => expect(useDevelopmentDrafts.getState().entries[pendingKey]).toBeUndefined());
  render(<RecoverableConversation />);
  expect(screen.getByLabelText<HTMLTextAreaElement>('agent.prompt').value).toBe('Retain this rejected prompt');
  expect(getConversationReferences('p', session.id)).toEqual([file]);
  expect(screen.getByRole('button', { name: 'agent.send' })).toHaveProperty('disabled', false);
});

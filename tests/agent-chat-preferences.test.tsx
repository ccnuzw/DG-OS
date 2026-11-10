import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { agentChatDisplayKey, agentChatPreferences, bindAgentChatPreferences, saveAgentChatPreferences } from '../src/stores/agentChatPreferences';
import { agentDraftSettingsKey, agentSettingsKey, bindAgentDraftSettings } from '../src/stores/agentTurnSettings';
import { desktopSlot, initializeDesktopState, resetDesktopState, setDesktopSlot } from '../src/stores/developmentDesktop';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { useAgentRunner, type AgentEvent, type AgentSession } from '../src/stores/agentRunnerStore';
import { AgentConversation } from '../src/pages/workbench/AgentConversation';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
const session: AgentSession = { id: 's', generation: 'g', engine: 'claude', native_id: 'native', workspace: '/repo', status: 'idle', events: [], pending: [], error: null };
let revision = 0;
beforeEach(async () => {
  revision = 0;
  vi.mocked(invoke).mockImplementation(async (name, args) => {
    if (name === 'read_development_desktop_state') return { revision: 0, value: {} };
    if (name === 'save_development_desktop_state' && args && typeof args === 'object' && 'input' in args && args.input && typeof args.input === 'object' && 'value' in args.input) return { revision: ++revision, value: args.input.value };
    return null;
  });
  await initializeDesktopState('account:a');
  useDevelopment.setState({ selection: { project_id: 'p', task_id: null, session_id: session.id } });
  useAgentRunner.setState({ selected: session.id, session, snapshots: { [session.id]: session } });
});
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); useDevelopmentDrafts.getState().clear(); resetDesktopState(); vi.mocked(invoke).mockReset(); });
it('persists response intent through existing turn settings and transfers only this draft display preferences', async () => {
  const draft = { projectId: 'p', draftId: 'draft-one' };
  expect(await saveAgentChatPreferences(draft, { language: 'zh', brevity: 'concise', stream_response: false })).toBe(true);
  expect(useDevelopmentDrafts.getState().entries[agentDraftSettingsKey('p', 'draft-one')].value).toMatchObject({ response_preferences: { language: 'zh', brevity: 'concise', next_step_suggestions: true } });
  bindAgentDraftSettings('p', 'draft-one', session); bindAgentChatPreferences('p', 'draft-one', session);
  expect(useDevelopmentDrafts.getState().entries[agentSettingsKey(session)].value).toMatchObject({ response_preferences: { language: 'zh' } });
  expect(agentChatPreferences({ projectId: 'p', session })).toMatchObject({ language: 'zh', stream_response: false });
  expect(agentChatPreferences({ projectId: 'p', draftId: 'another-draft' })).toMatchObject({ language: 'follow_input', stream_response: true });
  expect(desktopSlot(agentChatDisplayKey('p', 'draft-one'), null)).toBeNull();
});
it('rolls failed preference persistence back without touching composer intent', async () => {
  const key = agentSettingsKey(session);
  useDevelopmentDrafts.setState({ entries: { [key]: { kind: 'conversation', baseline: {}, value: { reasoning_effort: 'high' } } } });
  vi.mocked(invoke).mockRejectedValueOnce('disk unavailable');
  expect(await saveAgentChatPreferences({ projectId: 'p', session }, { language: 'en', follow_edits: false })).toBe(false);
  expect(useDevelopmentDrafts.getState().entries[key].value).toEqual({ reasoning_effort: 'high' });
  expect(agentChatPreferences({ projectId: 'p', session })).toMatchObject({ language: 'follow_input', follow_edits: true });
});
function show(value: AgentSession, onOpenReference = vi.fn()) {
  return <MemoryRouter><AgentConversation session={value} drafts={{}} setDraft={vi.fn()} busy={false} error={false} onOpenReference={onOpenReference} onOpenRuntime={vi.fn()} /></MemoryRouter>;
}
it('buffers current assistant text while keeping tools live and releases text on cancellation', () => {
  setDesktopSlot(agentChatDisplayKey('p', session.id), { stream_response: false });
  const events: AgentEvent[] = [{ seq: 1, kind: 'agentMessage', text: 'Previous answer', data: null, run: 'old' }, { seq: 2, kind: 'text_delta', text: 'Buffered answer', data: null, run: 'current' }, { seq: 3, kind: 'commandExecution', text: null, data: { type: 'commandExecution', command: 'pwd', status: 'inProgress' }, run: 'current' }];
  const running = { ...session, status: 'running', active_run: 'current', events };
  act(() => useAgentRunner.setState({ session: running }));
  const view = render(show(running));
  expect(screen.getByText('Previous answer')).toBeTruthy(); expect(screen.queryByText('Buffered answer')).toBeNull(); expect(screen.getByText('pwd')).toBeTruthy();
  const cancelled = { ...running, status: 'idle', active_run: null, events: [...events, { seq: 4, kind: 'result', text: null, data: { run_status: 'cancelled' }, run: 'current' }] };
  act(() => useAgentRunner.setState({ session: cancelled })); view.rerender(show(cancelled));
  expect(screen.getByText('Buffered answer')).toBeTruthy();
});
it('changes code rendering and hides suggestion actions without changing stored text', () => {
  const source = '```javascript\nconst answer = 42;\n```';
  const value = { ...session, events: [{ seq: 1, kind: 'agentMessage', text: source, data: null, run: 'r' }, { seq: 2, kind: 'result', text: null, data: { run_status: 'completed' }, run: 'r' }] };
  act(() => useAgentRunner.setState({ session: value }));
  const view = render(show(value)); expect(view.container.querySelector('.hljs-keyword')).toBeTruthy(); expect(screen.getByRole('button', { name: 'agent.runTests' })).toBeTruthy();
  act(() => { setDesktopSlot(agentChatDisplayKey('p', session.id), { code_highlighting: false }); useDevelopmentDrafts.setState({ entries: { [agentSettingsKey(session)]: { kind: 'conversation', baseline: {}, value: { response_preferences: { language: 'follow_input', brevity: 'standard', next_step_suggestions: false } } } } }); });
  expect(view.container.querySelector('.hljs-keyword')).toBeNull(); expect(screen.queryByRole('button', { name: 'agent.runTests' })).toBeNull(); expect(value.events[0].text).toBe(source);
});
it('follows newly observed AI edits but stops after manual document browsing', async () => {
  const running = { ...session, status: 'running', active_run: 'turn' }; act(() => useAgentRunner.setState({ session: running }));
  const open = vi.fn((_reference: unknown, _mode: unknown, _shouldOpen?: () => boolean) => {}); const view = render(<><aside className="dev-document-column"><button>Browse file</button></aside>{show(running, open)}</>);
  const edit = (seq: number): AgentEvent => ({ seq, kind: 'fileChange', text: null, run: 'turn', data: { type: 'fileChange', status: 'completed', changes: [{ path: `file-${seq}.ts` }] } });
  const next = { ...running, events: [edit(1)] }; act(() => useAgentRunner.setState({ session: next })); view.rerender(<><aside className="dev-document-column"><button>Browse file</button></aside>{show(next, open)}</>);
  await act(async () => { await Promise.resolve(); });
  expect(open).toHaveBeenCalledTimes(1); expect(open.mock.calls[0][2]?.()).toBe(true);
  fireEvent.pointerDown(screen.getByRole('button', { name: 'Browse file' }));
  const later = { ...next, events: [...next.events, edit(2)] }; act(() => useAgentRunner.setState({ session: later })); view.rerender(<><aside className="dev-document-column"><button>Browse file</button></aside>{show(later, open)}</>);
  await act(async () => { await Promise.resolve(); });
  expect(open).toHaveBeenCalledTimes(1); expect(open.mock.calls[0][2]?.()).toBe(false);
});

it('cancels an unresolved edit follow when follow edits is disabled', async () => {
  const running = { ...session, status: 'running', active_run: 'turn' };
  act(() => useAgentRunner.setState({ session: running }));
  const resolved = Promise.withResolvers<void>();
  let shouldOpen: (() => boolean) | undefined;
  const open = vi.fn((_reference: unknown, _mode: unknown, current?: () => boolean) => { shouldOpen = current; return resolved.promise; });
  const view = render(show(running, open));
  const next = { ...running, events: [{ seq: 1, kind: 'fileChange', text: null, run: 'turn', data: { type: 'fileChange', status: 'completed', changes: [{ path: 'edited.ts' }] } }] };
  act(() => useAgentRunner.setState({ session: next }));
  view.rerender(show(next, open));
  await act(async () => { await Promise.resolve(); });
  expect(shouldOpen?.()).toBe(true);
  act(() => setDesktopSlot(agentChatDisplayKey('p', session.id), { follow_edits: false }));
  expect(shouldOpen?.()).toBe(false);
  await act(async () => resolved.resolve());
});

it('edits response preferences by saved session id without importing current project execution defaults', async () => {
  setDesktopSlot('agent-defaults:p', { configuration: null, settings: { permission_mode: 'accept_edits', reasoning_effort: 'high' } });
  setDesktopSlot(`agent-session:${session.id}`, { configuration: null, settings: { permission_mode: 'read_only', reasoning_effort: 'low' } });
  expect(await saveAgentChatPreferences({ projectId: 'p', sessionId: session.id }, { language: 'en' })).toBe(true);
  expect(useDevelopmentDrafts.getState().entries[agentDraftSettingsKey('p', session.id)].value).toMatchObject({ permission_mode: 'read_only', reasoning_effort: 'low', response_preferences: { language: 'en' } });
});

it('keeps following new conversation messages when automatic file following is disabled', () => {
  setDesktopSlot(agentChatDisplayKey('p', session.id), { follow_edits: false });
  const view = render(show(session));
  const transcript = screen.getByRole('region', { name: 'agent.messages' });
  Object.defineProperty(transcript, 'scrollHeight', { configurable: true, value: 200 });
  const next = { ...session, events: [{ seq: 1, kind: 'agentMessage', text: 'A new response', data: null, run: 'turn' }] };
  act(() => useAgentRunner.setState({ session: next }));
  view.rerender(show(next));
  expect(transcript.scrollTop).toBe(200);
});

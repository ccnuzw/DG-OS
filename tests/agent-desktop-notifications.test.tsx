import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useAgentDesktopNotifications } from '../src/components/development/useAgentDesktopNotifications';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../src/i18n', () => {
  const t = (key: string) => key;
  return { useLocale: () => ({ t }), t };
});

class DesktopNotice {
  static permission: NotificationPermission = 'default';
  static instances: DesktopNotice[] = [];
  static requestPermission = vi.fn(async (): Promise<NotificationPermission> => { DesktopNotice.permission = 'granted'; return 'granted'; });
  onclick: (() => void) | null = null;
  onclose: (() => void) | null = null;
  close = vi.fn();
  constructor(readonly title: string, readonly options: NotificationOptions) { DesktopNotice.instances.push(this); }
}
const initial: AgentSession = { id: 's', generation: 'generation', engine: 'codex', native_id: null, workspace: '/private/workspace', status: 'running', events: [{ seq: 1, kind: 'userMessage', text: 'Private user message', data: null, run: 'run' }], pending: [], error: null, active_run: 'run' };
const terminal = { seq: 2, kind: 'turn/completed', text: 'Private assistant answer', data: { run_status: 'completed' }, run: 'run' };

beforeEach(() => {
  localStorage.clear();
  DesktopNotice.permission = 'default';
  DesktopNotice.instances = [];
  DesktopNotice.requestPermission.mockClear();
  vi.stubGlobal('Notification', DesktopNotice);
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
  vi.spyOn(window, 'focus').mockImplementation(() => {});
  useDevelopment.setState({ workspaceOwner: 'alice', sessions: [{ id: 's', owner: 'alice', project_id: null, task_id: null, engine: 'codex', native_id: null, workspace: '/private/workspace', model: null, executable: null }] });
  useAgentRunner.setState({ snapshots: { s: initial } });
});

afterEach(() => {
  cleanup();
  useDevelopment.getState().reset();
  useAgentRunner.getState().reset();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
});

it('requests permission only from explicit opt-in and stores an owner-scoped preference', async () => {
  const { result } = renderHook(() => useAgentDesktopNotifications({ onSelectSession: vi.fn() }));
  expect(result.current.enabled).toBe(false);
  expect(DesktopNotice.requestPermission).not.toHaveBeenCalled();
  await act(async () => { await result.current.setEnabled(true); });
  expect(DesktopNotice.requestPermission).toHaveBeenCalledTimes(1);
  expect(result.current.enabled).toBe(true);
  expect(localStorage.getItem('dgos:agent-notifications:alice')).toBe('true');
  expect(DesktopNotice.instances).toHaveLength(0);
});

it('notifies new approvals and exact completion evidence once without message bodies, then opens the session', async () => {
  const select = vi.fn();
  const { result } = renderHook(() => useAgentDesktopNotifications({ onSelectSession: select }));
  await act(async () => { await result.current.setEnabled(true); });
  const approval = { ...initial, status: 'waiting', pending: [{ id: 'approval', kind: 'command', title: 'Secret command title', data: { command: 'Secret command' } }] };
  act(() => { useAgentRunner.setState({ snapshots: { s: approval } }); });
  expect(DesktopNotice.instances).toHaveLength(1);
  expect(DesktopNotice.instances[0].title).toBe('workbenchNavigation.notificationApproval');
  act(() => { useAgentRunner.setState({ snapshots: { s: { ...approval } } }); });
  expect(DesktopNotice.instances).toHaveLength(1);
  const finished = { ...initial, status: 'idle', active_run: null, events: [...initial.events, terminal] };
  act(() => { useAgentRunner.setState({ snapshots: { s: finished } }); });
  act(() => { useAgentRunner.setState({ snapshots: { s: { ...finished } } }); });
  expect(DesktopNotice.instances).toHaveLength(2);
  expect(DesktopNotice.instances[1].title).toBe('workbenchNavigation.notificationCompleted');
  for (const notice of DesktopNotice.instances) {
    expect(notice.options.body).toBeUndefined();
    expect(JSON.stringify(notice.options)).not.toMatch(/Private|Secret|workspace/);
  }
  act(() => { DesktopNotice.instances[1].onclick?.(); });
  expect(select).toHaveBeenCalledWith('s');
  expect(window.focus).toHaveBeenCalledTimes(1);
});

it('does not replay historical completion on enabling or fabricate completion from idle status', async () => {
  useAgentRunner.setState({ snapshots: { s: { ...initial, events: [...initial.events, terminal], status: 'idle' } } });
  const { result } = renderHook(() => useAgentDesktopNotifications({ onSelectSession: vi.fn() }));
  await act(async () => { await result.current.setEnabled(true); });
  act(() => { useAgentRunner.setState({ snapshots: { s: { ...initial, status: 'idle' } } }); });
  expect(DesktopNotice.instances).toHaveLength(0);
});

it('suppresses the focused conversation and stops delivering after opt-out', async () => {
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  const { result } = renderHook(() => useAgentDesktopNotifications({ onSelectSession: vi.fn(), activeSessionId: 's' }));
  await act(async () => { await result.current.setEnabled(true); });
  act(() => { useAgentRunner.setState({ snapshots: { s: { ...initial, events: [...initial.events, terminal] } } }); });
  expect(DesktopNotice.instances).toHaveLength(0);
  await act(async () => { await result.current.setEnabled(false); });
  act(() => { useAgentRunner.setState({ snapshots: { s: { ...initial, pending: [{ id: 'later', kind: 'command', title: 'Private', data: null }] } } }); });
  expect(DesktopNotice.instances).toHaveLength(0);
  expect(localStorage.getItem('dgos:agent-notifications:alice')).toBe('false');
});

it('closes stale account notifications and refuses their click handlers', async () => {
  const select = vi.fn();
  const { result } = renderHook(() => useAgentDesktopNotifications({ onSelectSession: select }));
  await act(async () => { await result.current.setEnabled(true); });
  act(() => { useAgentRunner.setState({ snapshots: { s: { ...initial, events: [...initial.events, terminal] } } }); });
  const notice = DesktopNotice.instances[0];
  act(() => { useDevelopment.setState(state => ({ workspaceOwner: 'bob', epoch: state.epoch + 1 })); });
  expect(notice.close).toHaveBeenCalled();
  act(() => { notice.onclick?.(); });
  expect(select).not.toHaveBeenCalled();
  expect(result.current.enabled).toBe(false);
});

it('does not enable delivery when permission is denied or the platform has no notifications', async () => {
  DesktopNotice.permission = 'denied';
  const first = renderHook(() => useAgentDesktopNotifications({ onSelectSession: vi.fn() }));
  await act(async () => { await first.result.current.setEnabled(true); });
  expect(first.result.current.permission).toBe('denied');
  expect(first.result.current.enabled).toBe(false);
  expect(DesktopNotice.requestPermission).not.toHaveBeenCalled();
  first.unmount();
  vi.stubGlobal('Notification', undefined);
  const second = renderHook(() => useAgentDesktopNotifications({ onSelectSession: vi.fn() }));
  expect(second.result.current.supported).toBe(false);
  expect(second.result.current.permission).toBe('unsupported');
});

it('keeps notifications disabled when opt-out overtakes an open permission prompt', async () => {
  const permission = Promise.withResolvers<NotificationPermission>();
  DesktopNotice.requestPermission.mockImplementationOnce(() => permission.promise);
  const { result } = renderHook(() => useAgentDesktopNotifications({ onSelectSession: vi.fn() }));
  const enabling = result.current.setEnabled(true);
  await act(async () => { await result.current.setEnabled(false); });
  await act(async () => { DesktopNotice.permission = 'granted'; permission.resolve('granted'); await enabling; });
  expect(result.current.enabled).toBe(false);
  expect(localStorage.getItem('dgos:agent-notifications:alice')).toBe('false');
});

it('activates the hidden workbench before selecting the notified conversation', async () => {
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false);
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  const calls: string[] = [];
  let activeSurface = 'skills';
  const activate = vi.fn(() => { activeSurface = 'agent'; calls.push('activate'); });
  const select = vi.fn((id: string) => { expect(activeSurface).toBe('agent'); calls.push(id); });
  const { result } = renderHook(() => useAgentDesktopNotifications({ onActivate: activate, onSelectSession: select, activeSessionId: null }));
  await act(async () => { await result.current.setEnabled(true); });
  act(() => { useAgentRunner.setState({ snapshots: { s: { ...initial, events: [...initial.events, terminal] } } }); });
  expect(activate).not.toHaveBeenCalled();
  expect(select).not.toHaveBeenCalled();
  act(() => { DesktopNotice.instances[0].onclick?.(); });
  expect(calls).toEqual(['activate', 's']);
});

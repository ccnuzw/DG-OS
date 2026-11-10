import { afterEach, expect, it, vi } from 'vitest';
import { useDevelopment, type DevelopmentSession } from '../src/stores/developmentStore';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { invoke } from '@tauri-apps/api/core';
import { flushDesktopState } from '../src/stores/developmentDesktop';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const session: DevelopmentSession = { id: 'session', owner: 'user-a', project_id: 'project', task_id: 'task', engine: 'codex', native_id: 'native', workspace: '/fixture', executable: null, model: 'model' };
function workspace(owner = 'user-a') { return { projects: [{ id: 'project', owner, name: 'Project', path: '/fixture', directory_identity: 'directory', archived: false, created_at: 1, last_opened_at: 1 }], tasks: [], sessions: [{ ...session, owner }], selection: { project_id: 'project', task_id: null, session_id: null }, operations: [] }; }
interface Snapshot { revision: number; value: Record<string, unknown> }
interface SaveArgs { input: { expected_revision: number; value: Record<string, unknown> } }
const snapshots = new Map<string, Snapshot>();
const hostCommands: Record<string, true> = { read_development_workspace: true, read_development_desktop_state: true, save_development_desktop_state: true };
function installWorkspace(data: unknown = workspace(), owner = 'user-a') {
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === 'read_development_workspace') return data;
    const saved = snapshots.get(owner) ?? { revision: 0, value: {} };
    if (command === 'read_development_desktop_state') return structuredClone(saved);
    if (command === 'save_development_desktop_state') {
      const request = args as SaveArgs;
      if (saved.revision !== request.input.expected_revision) throw 'desktop_state_conflict';
      const next = { revision: saved.revision + 1, value: structuredClone(request.input.value) };
      snapshots.set(owner, next); return structuredClone(next);
    }
    throw new Error(`Unexpected host command: ${command}`);
  });
}
afterEach(() => { useDevelopment.getState().reset(); useAgentRunner.getState().reset(); snapshots.clear(); localStorage.clear(); vi.mocked(invoke).mockReset(); });
it('deduplicates history tabs and preserves a draft tab identity when the first session is admitted', () => {
  const state = useDevelopment.getState();
  const draft = state.newTab('project');
  state.renameTab(draft.id, 'Named draft');
  state.openTab(session);
  state.bindTab(draft.id, session);
  const reopened = state.openTab(session);
  expect(reopened?.id).toBe(draft.id);
  expect(useDevelopment.getState().tabs.map(tab => tab.sessionId)).toEqual(['session']);
  expect(reopened?.title).toBe('Named draft');
});
it('restores account-owned workspace and unsent drafts without invoking runtime or replaying sends', async () => {
  installWorkspace();
  await useDevelopment.getState().load();
  const state = useDevelopment.getState();
  const tab = state.openTab(session)!;
  state.renameTab(tab.id, 'Durable name');
  state.setReading(session.id, { top: 240, follow: false });
  useDevelopmentDrafts.setState({ entries: { [developmentDraftKey('conversation', 'project', session.id, 'message')]: { kind: 'conversation', value: 'unsent text', baseline: '' } } });
  expect(await flushDesktopState()).toBe(true); state.reset();
  await useDevelopment.getState().load();
  expect(useDevelopment.getState().selection.session_id).toBe(session.id);
  expect(useDevelopment.getState().tabs[0].title).toBe('Durable name');
  expect(useDevelopment.getState().reading.session).toEqual({ top: 240, follow: false });
  expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', 'project', session.id, 'message')].value).toBe('unsent text');
  expect(vi.mocked(invoke).mock.calls.every(([command]) => hostCommands[command])).toBe(true);
  state.reset(); installWorkspace(workspace('user-b'), 'user-b');
  await useDevelopment.getState().load();
  expect(useDevelopment.getState().tabs).toEqual([]);
  expect(useDevelopment.getState().sessionNames).toEqual({});
});
it('keeps closed tabs closed across restart and keeps names available for explicit history reopening', async () => {
  installWorkspace();
  await useDevelopment.getState().load();
  const state = useDevelopment.getState();
  const tab = state.openTab(session)!;
  state.renameTab(tab.id, 'History name'); state.closeTab(tab.id); await flushDesktopState(); state.reset();
  await useDevelopment.getState().load();
  expect(useDevelopment.getState().selection).toEqual({ project_id: null, task_id: null, session_id: null });
  expect(useDevelopment.getState().tabs).toEqual([]);
  expect(useDevelopment.getState().openTab(session)?.title).toBe('History name');
});
it('drops a workspace load that arrives after an account reset', async () => {
  const pending = Promise.withResolvers<unknown>();
  vi.mocked(invoke).mockReturnValue(pending.promise);
  const loading = useDevelopment.getState().load();
  useDevelopment.getState().reset(); pending.resolve(workspace());
  expect(await loading).toBe(false);
  expect(useDevelopment.getState().tabs).toEqual([]);
  expect(useDevelopment.getState().workspaceOwner).toBeNull();
});
it('retains a removed project tab as unavailable metadata instead of selecting another project', async () => {
  installWorkspace();
  await useDevelopment.getState().load('alice');
  useDevelopment.getState().openTab(session);
  await flushDesktopState(); useDevelopment.getState().reset();
  installWorkspace({ projects: [], tasks: [], sessions: [], operations: [], selection: { project_id: null, task_id: null, session_id: null } });
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().tabs[0].sessionId).toBe('session');
  expect(useDevelopment.getState().selection.project_id).toBe('project');
  expect(useDevelopment.getState().projects).toEqual([]);
  expect(vi.mocked(invoke).mock.calls.every(([command]) => hostCommands[command])).toBe(true);
});
it('persists project pins and archived history through refresh, account reset and recovery', async () => {
  const data = workspace();
  installWorkspace(data);
  await useDevelopment.getState().load('alice');
  useDevelopment.getState().setProjectPinned('project', true);
  useDevelopment.getState().setProjectPinned('project', true);
  useDevelopment.getState().setProjectPinned('missing', true);
  useDevelopment.getState().archiveSessions(['session', 'session', 'missing', '']);
  expect(useDevelopment.getState().pinnedProjectIds).toEqual(['project']);
  expect(useDevelopment.getState().archivedSessionIds).toEqual(['session']);
  expect(useDevelopment.getState().sessions).toEqual(data.sessions);
  expect(useDevelopment.getState().selection).toEqual(data.selection);
  expect(useDevelopment.getState().tabs).toEqual([]);
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual(['project']);
  expect(useDevelopment.getState().archivedSessionIds).toEqual(['session']);
  await flushDesktopState(); useDevelopment.getState().reset();
  expect(useDevelopment.getState().pinnedProjectIds).toEqual([]);
  expect(useDevelopment.getState().archivedSessionIds).toEqual([]);
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual(['project']);
  expect(useDevelopment.getState().archivedSessionIds).toEqual(['session']);
  expect(useDevelopment.getState().sessions).toEqual(data.sessions);
  useDevelopment.getState().restoreSessions(['session', 'session', 'missing', '']);
  useDevelopment.getState().setProjectPinned('project', false);
  await flushDesktopState(); useDevelopment.getState().reset();
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual([]);
  expect(useDevelopment.getState().archivedSessionIds).toEqual([]);
  expect(useDevelopment.getState().sessions).toEqual(data.sessions);
  expect(vi.mocked(invoke).mock.calls.every(([command]) => hostCommands[command])).toBe(true);
});
it('isolates pin and archive metadata between owners even when loading another account directly', async () => {
  installWorkspace();
  await useDevelopment.getState().load('alice');
  useDevelopment.getState().setProjectPinned('project', true);
  useDevelopment.getState().archiveSessions(['session']);
  await flushDesktopState(); installWorkspace(workspace('user-b'), 'user-b');
  await useDevelopment.getState().load('bob');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual([]);
  expect(useDevelopment.getState().archivedSessionIds).toEqual([]);
  useDevelopment.getState().reset();
  installWorkspace();
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual(['project']);
  expect(useDevelopment.getState().archivedSessionIds).toEqual(['session']);
  useDevelopment.getState().reset();
  installWorkspace(workspace('user-b'), 'user-b');
  await useDevelopment.getState().load('bob');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual([]);
  expect(useDevelopment.getState().archivedSessionIds).toEqual([]);
});
it('restores old saved workspaces and validates deduplicated pin and archive ids', async () => {
  installWorkspace();
  localStorage.setItem('dgos.development.conversations:account:alice', JSON.stringify({ tabs: [], activeTabId: null, sessionNames: {}, reading: {} }));
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual([]);
  expect(useDevelopment.getState().archivedSessionIds).toEqual([]);
  useDevelopment.getState().reset();
  localStorage.setItem('dgos.development.conversations:account:alice', JSON.stringify({ tabs: [], pinnedProjectIds: ['project', 'project', null, 4, '', '   '], archivedSessionIds: ['session', false, {}, 'session', '', '\t'] }));
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual(['project']);
  expect(useDevelopment.getState().archivedSessionIds).toEqual(['session']);
  useDevelopment.getState().reset();
  localStorage.setItem('dgos.development.conversations:account:alice', JSON.stringify({ tabs: [], pinnedProjectIds: 'project', archivedSessionIds: { session: true } }));
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual([]);
  expect(useDevelopment.getState().archivedSessionIds).toEqual([]);
});
it('archives only closed history while preserving active tabs, selected history and live runtimes', async () => {
  const ids = ['current', 'open', 'live', 'starting', 'running', 'closed', 'history'];
  const sessions = ids.map(id => ({ ...session, id }));
  const selection = { project_id: 'project', task_id: null, session_id: 'current' };
  installWorkspace({ ...workspace(), sessions, selection });
  await useDevelopment.getState().load();
  const tab = useDevelopment.getState().openTab(sessions[1]);
  const snapshots = Object.fromEntries(['starting', 'running', 'closed'].map(id => [id, { ...session, id, generation: 'generation', status: id, events: [], pending: [], error: null } satisfies AgentSession]));
  useAgentRunner.setState({ live: { live: 'waiting', closed: 'closed' }, snapshots });
  const runner = useAgentRunner.getState();
  useDevelopment.getState().archiveSessions([...ids, 'history', 'missing', '', '   ']);
  expect(useDevelopment.getState().archivedSessionIds).toEqual(['closed', 'history']);
  expect(useDevelopment.getState().sessions).toEqual(sessions);
  expect(useDevelopment.getState().selection).toEqual(selection);
  expect(useDevelopment.getState().tabs).toEqual([tab]);
  expect(useAgentRunner.getState()).toBe(runner);
  useDevelopment.getState().restoreSessions(['history', 'closed']);
  expect(useDevelopment.getState().archivedSessionIds).toEqual([]);
  expect(useDevelopment.getState().sessions).toEqual(sessions);
  expect(vi.mocked(invoke).mock.calls.every(([command]) => hostCommands[command])).toBe(true);
});
it('retains navigation metadata when a project becomes unavailable, matching persisted conversation tabs', async () => {
  installWorkspace();
  await useDevelopment.getState().load('alice');
  useDevelopment.getState().setProjectPinned('project', true);
  useDevelopment.getState().archiveSessions(['session']);
  await flushDesktopState(); useDevelopment.getState().reset();
  installWorkspace({ projects: [], tasks: [], sessions: [], operations: [], selection: { project_id: null, task_id: null, session_id: null } });
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().pinnedProjectIds).toEqual(['project']);
  expect(useDevelopment.getState().archivedSessionIds).toEqual(['session']);
  useDevelopment.getState().setProjectPinned('project', false);
  useDevelopment.getState().restoreSessions(['session']);
  expect(useDevelopment.getState().pinnedProjectIds).toEqual([]);
  expect(useDevelopment.getState().archivedSessionIds).toEqual([]);
});

it('retains even empty closed drafts and restores them explicitly after restart', async () => {
  installWorkspace(); await useDevelopment.getState().load();
  const draft = useDevelopment.getState().newTab('project');
  useDevelopment.getState().closeTab(draft.id);
  expect(useDevelopment.getState().activeTabId).toBeNull();
  expect(useDevelopment.getState().tabs).toEqual([{ ...draft, hidden: true }]);
  await flushDesktopState(); useDevelopment.getState().reset(); await useDevelopment.getState().load();
  expect(useDevelopment.getState().tabs).toEqual([{ ...draft, hidden: true }]);
  expect(useDevelopment.getState().activeTabId).toBeNull();
  useDevelopment.getState().activateTab(draft.id);
  expect(useDevelopment.getState().tabs[0].hidden).toBe(false);
  expect(useDevelopment.getState().activeTabId).toBe(draft.id);
});

it('keeps the legacy migration source until the host save succeeds', async () => {
  installWorkspace();
  const key = 'dgos.development.conversations:account:alice';
  const legacy = JSON.stringify({ tabs: [], activeTabId: null, sessionNames: { session: 'Legacy title' }, reading: {} });
  localStorage.setItem(key, legacy);
  await useDevelopment.getState().load('alice');
  expect(useDevelopment.getState().sessionNames.session).toBe('Legacy title');
  vi.mocked(invoke).mockRejectedValueOnce('disk full');
  expect(await flushDesktopState()).toBe(false); expect(localStorage.getItem(key)).toBe(legacy);
  expect(await flushDesktopState()).toBe(true); expect(localStorage.getItem(key)).toBeNull();
});

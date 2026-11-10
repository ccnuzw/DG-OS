import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { act, cleanup, renderHook } from '@testing-library/react';
import { desktopSlot, flushDesktopState, initializeDesktopState, reloadDesktopState, retryDesktopState, setDesktopSlot, useDesktopPersistence } from '../src/stores/developmentDesktop';
import { useDevelopment } from '../src/stores/developmentStore';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { agentDraftSettingsKey, agentSettingsKey, agentTurnSettings, bindAgentDraftSettings, rememberAgentSessionConfiguration, resolveAgentDefaults, setAgentDefaults } from '../src/stores/agentTurnSettings';
import { agentImagesKey, captureAgentImages, useAgentImages, AGENT_IMAGE_LIFETIME_MS } from '../src/stores/agentImages';
import { agentSkillsKey, captureAgentSkills } from '../src/stores/agentSkills';
import { addConversationReference, captureConversationReferences } from '../src/stores/developmentReferences';
import { documentId, openDevelopmentDocument, useDevelopmentEditor } from '../src/stores/developmentEditor';
import type { AgentSession } from '../src/stores/agentRunnerStore';
import { useDevelopmentRuntime, useRuntimeDraft } from '../src/stores/developmentRuntime';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key, useLocale: () => ({ t: (key: string) => key }) }));
interface SavedState { revision: number; value: Record<string, unknown> }
interface DesktopSaveArgs { input: { expected_revision: number; value: Record<string, unknown> } }
let saved: SavedState;
const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value));
beforeEach(() => {
  saved = { revision: 0, value: {} };
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === 'read_development_desktop_state') return clone(saved);
    if (command === 'save_development_desktop_state') {
      const request = args as DesktopSaveArgs; // In-process calls made by this test's mocked host boundary.
      const input = request.input;
      if (input.expected_revision !== saved.revision) throw 'desktop_state_conflict';
      saved = { revision: saved.revision + 1, value: clone(input.value) }; return clone(saved);
    }
    if (command === 'read_development_file') return { path: 'a.ts', text: 'base', hash: 'initial', size: 4, reason: null };
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(() => { cleanup(); useDevelopment.getState().reset(); vi.mocked(invoke).mockReset(); vi.useRealTimers(); });
const session: AgentSession = { id: 'session', generation: 'first-generation', engine: 'codex', native_id: 'native', workspace: '/repo', status: 'idle', events: [], pending: [], error: null };

it('reads before saving, merges drafts typed during loading, and never overwrites unseen drafts', async () => {
  const pending = Promise.withResolvers<SavedState>();
  const old = JSON.stringify([12, 'conversation', 'p', 'old', 'message']);
  const local = developmentDraftKey('conversation', 'p', 'new', 'message');
  vi.mocked(invoke).mockImplementationOnce(() => pending.promise);
  const initializing = initializeDesktopState('alice');
  useDevelopmentDrafts.setState({ entries: { [local]: { kind: 'conversation', value: 'typed during read', baseline: '' } } });
  const flushing = flushDesktopState();
  expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toEqual(['read_development_desktop_state']);
  pending.resolve({ revision: 0, value: { drafts: { entries: { [old]: { kind: 'conversation', value: 'yesterday', baseline: '' } }, handoffs: {} } } });
  await initializing; expect(await flushing).toBe(true);
  expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', 'p', 'old', 'message')].value).toBe('yesterday');
  expect(useDevelopmentDrafts.getState().entries[local].value).toBe('typed during read');
  expect(JSON.stringify(saved.value)).toContain('yesterday');
  expect(JSON.stringify(saved.value)).toContain('typed during read');
});

it('exposes failed reads and retains changes until a successful explicit retry', async () => {
  vi.mocked(invoke).mockRejectedValueOnce('disk unavailable');
  await initializeDesktopState('alice'); setDesktopSlot('view', { selected: 'a' });
  expect(useDesktopPersistence.getState()).toMatchObject({ loaded: false, dirty: true, error: 'desktop_state_save_failed' });
  expect(await flushDesktopState()).toBe(false);
  expect(vi.mocked(invoke).mock.calls).toHaveLength(1);
  expect(await retryDesktopState()).toBe(true);
  expect(saved.value.view).toEqual({ selected: 'a' });
});

it('drains concurrent edits through one save flight and the latest revision', async () => {
  await initializeDesktopState('alice'); await flushDesktopState();
  const pending = Promise.withResolvers<SavedState>();
  vi.mocked(invoke).mockImplementationOnce(() => pending.promise);
  setDesktopSlot('view', 'first'); const first = flushDesktopState();
  setDesktopSlot('view', 'second'); const second = flushDesktopState();
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'save_development_desktop_state')).toHaveLength(2); // initial hydration + current flight
  saved = { revision: saved.revision + 1, value: { ...saved.value, view: 'first' } }; pending.resolve(clone(saved));
  expect(await first).toBe(true); expect(await second).toBe(true);
  expect(saved.value.view).toBe('second'); expect(useDesktopPersistence.getState().dirty).toBe(false);
});

it('keeps CAS conflicts visible without retrying or replacing newer host state', async () => {
  await initializeDesktopState('alice'); await flushDesktopState();
  saved = { revision: saved.revision + 1, value: { ...saved.value, remoteOnly: 'kept' } };
  setDesktopSlot('view', 'local'); expect(await flushDesktopState()).toBe(false);
  expect(useDesktopPersistence.getState()).toMatchObject({ dirty: true, conflict: true, error: 'desktop_state_conflict' });
  const count = vi.mocked(invoke).mock.calls.length;
  expect(await retryDesktopState()).toBe(false); expect(vi.mocked(invoke).mock.calls).toHaveLength(count);
  expect(saved.value.view).toBeUndefined();
  await reloadDesktopState(); expect(await flushDesktopState()).toBe(true);
  expect(saved.value).toMatchObject({ view: 'local', remoteOnly: 'kept' });
});

it('ignores a previous account read after reset and never restores runtime execution', async () => {
  const old = Promise.withResolvers<SavedState>(); vi.mocked(invoke).mockImplementationOnce(() => old.promise);
  const reading = initializeDesktopState('alice'); useDevelopment.getState().reset();
  await initializeDesktopState('bob');
  old.resolve({ revision: 9, value: { view: 'private old account' } }); await reading;
  expect(desktopSlot('view', null)).toBeNull(); expect(useDesktopPersistence.getState().owner).toBe('bob');
  expect(vi.mocked(invoke).mock.calls.every(([command]) => command === 'read_development_desktop_state')).toBe(true);
});

it('rebases the renderer epoch while retaining unknown request identity and clearing transient preparing', async () => {
  const oldKey = JSON.stringify([99, 'conversation', 'p', 'draft']);
  saved.value = { drafts: { entries: { [oldKey]: { kind: 'conversation', baseline: {}, value: { prompt: 'keep me', phase: 'send_unknown', preparing: true, requestId: 'original', submittedGeneration: 'native-generation' } } }, handoffs: {} } };
  await initializeDesktopState('alice');
  const value = useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', 'p', 'draft')].value;
  expect(value).toEqual({ prompt: 'keep me', phase: 'send_unknown', preparing: false, requestId: 'original', submittedGeneration: 'native-generation' });
  expect(vi.mocked(invoke).mock.calls.every(([command]) => command === 'read_development_desktop_state')).toBe(true);
});

it('freezes original session configuration while new drafts inherit explicit project and application defaults', async () => {
  await initializeDesktopState('alice');
  const application = { engine: 'codex' as const, provider: { provider_id: 'one', model_id: 'app', provider_revision: 1 } };
  const project = { ...application, provider: { ...application.provider, model_id: 'project' } };
  setAgentDefaults('application', { configuration: application, settings: { permission_mode: 'accept_edits', reasoning_effort: 'medium' } });
  expect(resolveAgentDefaults('new-project')).toMatchObject({ source: 'application', settings: { permission_mode: 'default' } });
  setAgentDefaults('p', { configuration: project, settings: { collaboration_mode: 'plan' } });
  rememberAgentSessionConfiguration('p', session.id, project);
  useDevelopment.setState({ selection: { project_id: 'p', task_id: null, session_id: session.id } });
  useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentDraftSettingsKey('p', 'draft')]: { kind: 'conversation', value: { reasoning_effort: 'high' }, baseline: {} } } }));
  bindAgentDraftSettings('p', 'draft', session);
  const key = agentSettingsKey(session);
  expect(agentSettingsKey({ ...session, generation: 'next-generation' })).toBe(key);
  expect(agentTurnSettings(session)).toEqual({ reasoning_effort: 'high' });
  setAgentDefaults('p', { configuration: application, settings: {} });
  expect(resolveAgentDefaults('p', session.id).configuration).toEqual(project);
  expect(resolveAgentDefaults('p').configuration).toEqual(application);
  await flushDesktopState(); useDevelopment.getState().reset(); await initializeDesktopState('alice');
  useDevelopment.setState({ selection: { project_id: 'p', task_id: null, session_id: session.id } });
  expect(agentTurnSettings({ ...session, generation: 'restarted' })).toEqual({ reasoning_effort: 'high' });
});

it('retains reference hashes and skill identities across restart without treating them as validated capabilities', async () => {
  await initializeDesktopState('alice');
  addConversationReference('p', session.id, { project_id: 'p', task_id: 't', path: 'a.ts', kind: 'file', expected_hash: 'original-hash' });
  useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSkillsKey('p', session)]: { kind: 'conversation', baseline: [], value: [{ id: 'chosen', skill: { name: 'review', path: '/old/SKILL.md' } }] } } }));
  expect(await flushDesktopState()).toBe(true); useDevelopment.getState().reset(); await initializeDesktopState('alice');
  expect(captureConversationReferences('p', session.id)[0].reference.expected_hash).toBe('original-hash');
  expect(captureAgentSkills('p', { ...session, generation: 'next' })[0].skill.path).toBe('/old/SKILL.md');
  expect(vi.mocked(invoke).mock.calls.some(([command]) => /send_agent|observe_native/.test(command))).toBe(false);
});

it('restores bounded private image bytes and exposes expiration rather than silently removing attachment history', async () => {
  const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6bXkAAAAASUVORK5CYII=';
  const owner = agentImagesKey('p', session);
  saved.value = { images: { [owner]: { images: [{ id: 'image', name: 'image.png', bytes: atob(png).length, input: { mime: 'image/png', base64: png } }], busy: false, error: null, expiresAt: Date.now() + AGENT_IMAGE_LIFETIME_MS } } };
  await initializeDesktopState('alice');
  expect(captureAgentImages(agentImagesKey('p', { ...session, generation: 'next' }))[0].input.base64).toBe(png);
  expect(await flushDesktopState()).toBe(true); useDevelopment.getState().reset();
  vi.useFakeTimers(); vi.advanceTimersByTime(AGENT_IMAGE_LIFETIME_MS + 1); await initializeDesktopState('alice');
  const restored = agentImagesKey('p', session);
  expect(captureAgentImages(restored)).toEqual([]); expect(useAgentImages.getState().drafts[restored].error).toBe('expired');
});

it('restores unsaved editor text without writing disk and detects changed disk before an explicit save', async () => {
  await initializeDesktopState('alice');
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.getState().edit('unsaved content');
  openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path: 'a.ts', mode: 'file', source: 'task' });
  expect(await flushDesktopState()).toBe(true); useDevelopment.getState().reset(); await initializeDesktopState('alice');
  const id = documentId('p', 't', 'a.ts', '/repo');
  expect(useDevelopmentEditor.getState().documents[id]).toMatchObject({ draft: 'unsaved content', restored: true, saving: false, busy: false });
  await useDevelopmentEditor.getState().save(id);
  expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'save_development_file')).toBe(false);
  vi.mocked(invoke).mockResolvedValueOnce({ path: 'a.ts', text: 'someone else changed it', hash: 'changed', size: 23, reason: null });
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  expect(useDevelopmentEditor.getState().documents[id]).toMatchObject({ draft: 'unsaved content', error: 'file_changed', disk: { hash: 'changed' } });
  expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'save_development_file')).toBe(false);
});

it('retains edits on a failed save, reports failure, and retries only on an explicit request', async () => {
  await initializeDesktopState('alice'); await flushDesktopState();
  vi.useFakeTimers(); vi.mocked(invoke).mockRejectedValueOnce('disk full');
  setDesktopSlot('view', { content: 'keep on failure' });
  expect(await flushDesktopState()).toBe(false);
  expect(useDesktopPersistence.getState()).toMatchObject({ saving: false, dirty: true, error: 'desktop_state_save_failed' });
  const calls = vi.mocked(invoke).mock.calls.length;
  setDesktopSlot('view', { content: 'newer edit after failure' }); vi.advanceTimersByTime(1000);
  expect(vi.mocked(invoke).mock.calls).toHaveLength(calls);
  expect(await retryDesktopState()).toBe(true);
  expect(saved.value.view).toEqual({ content: 'newer edit after failure' });
});

it('debounces repeated edits into one bounded save of the latest value', async () => {
  await initializeDesktopState('alice'); await flushDesktopState(); vi.useFakeTimers();
  const before = vi.mocked(invoke).mock.calls.filter(([command]) => command === 'save_development_desktop_state').length;
  setDesktopSlot('view', 'one'); setDesktopSlot('view', 'two'); setDesktopSlot('view', 'three');
  vi.advanceTimersByTime(349);
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'save_development_desktop_state')).toHaveLength(before);
  vi.advanceTimersByTime(1); expect(await flushDesktopState()).toBe(true);
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'save_development_desktop_state')).toHaveLength(before + 1);
  expect(saved.value.view).toBe('three');
  setDesktopSlot('view', 'x'.repeat(8 * 1024 * 1024));
  expect(await flushDesktopState()).toBe(false);
  expect(useDesktopPersistence.getState()).toMatchObject({ dirty: true, error: 'desktop_state_limit' });
  expect(saved.value.view).toBe('three');
});

it('preserves locally edited navigation when reloading a conflicting desktop revision', async () => {
  await initializeDesktopState('alice');
  useDevelopment.setState({ workspaceOwner: 'alice', loaded: true });
  const first = useDevelopment.getState().newTab('p');
  expect(await flushDesktopState()).toBe(true);
  const second = useDevelopment.getState().newTab('p');
  saved = { revision: saved.revision + 1, value: { ...saved.value, remoteOnly: true } };
  expect(await flushDesktopState()).toBe(false);
  await reloadDesktopState();
  expect(useDevelopment.getState().tabs.map(tab => tab.id)).toEqual([first.id, second.id]);
  expect(useDevelopment.getState().activeTabId).toBe(second.id);
  expect(await flushDesktopState()).toBe(true);
  expect(saved.value.remoteOnly).toBe(true);
});

it('restores command drafts without restoring processes or dispatching execution', async () => {
  await initializeDesktopState('alice');
  const view = renderHook(() => useRuntimeDraft('p', 't'));
  act(() => {
    view.result.current.setField('program', 'npm');
    view.result.current.setField('args', ['run', 'test']);
    view.result.current.setField('cwd', 'packages/app');
    view.result.current.setField('env', [{ target: 'API_KEY', source: 'PROJECT_KEY' }]);
  });
  expect(await flushDesktopState()).toBe(true);
  view.unmount();
  useDevelopment.getState().reset();
  const before = vi.mocked(invoke).mock.calls.length;
  await initializeDesktopState('alice');
  const restored = renderHook(() => useRuntimeDraft('p', 't'));
  expect(restored.result.current.draft).toMatchObject({ dirty: true, program: 'npm', args: ['run', 'test'], cwd: 'packages/app', env: [{ target: 'API_KEY', source: 'PROJECT_KEY' }] });
  expect(useDevelopmentRuntime.getState()).toMatchObject({ taskId: null, processes: [], operations: [] });
  expect(vi.mocked(invoke).mock.calls.slice(before).map(([command]) => command)).toEqual(['read_development_desktop_state']);
});

it('rejects command draft callbacks captured before an account reset', () => {
  const view = renderHook(() => useRuntimeDraft('p', 't'));
  const stale = view.result.current.setField;
  act(() => { useDevelopment.getState().reset(); });
  act(() => { stale('program', 'private old account'); });
  expect(useDevelopmentRuntime.getState().drafts).toEqual({});
  expect(JSON.stringify(useDesktopPersistence.getState().value)).not.toContain('private old account');
});

it('recovers an interrupted workspace task preparation as unknown instead of allowing creation replay', async () => {
  const key = JSON.stringify([99, 'task', 'p', 'tab', 'workspace-preparation']);
  saved.value = { drafts: { entries: { [key]: { kind: 'task', value: { phase: 'pending' }, baseline: {} } }, handoffs: {} } };
  await initializeDesktopState('alice');
  expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('task', 'p', 'tab', 'workspace-preparation')].value).toEqual({ phase: 'unknown' });
  expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).toEqual(['read_development_desktop_state']);
});

it('classifies the host invalid desktop receipt as a non-retriable invalid state', async () => {
  await initializeDesktopState('alice'); await flushDesktopState();
  setDesktopSlot('view', 'unsaved');
  vi.mocked(invoke).mockRejectedValueOnce('invalid_desktop_state');
  expect(await flushDesktopState()).toBe(false);
  expect(useDesktopPersistence.getState().error).toBe('desktop_state_invalid');
});

it('keeps the same account epoch and drafts when an empty workspace gains its first project', async () => {
  let projects: { id: string; owner: string; name: string; path: string; directory_identity: string; archived: boolean; created_at: number; last_opened_at: null }[] = [];
  const implementation = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args) => command === 'read_development_workspace'
    ? Promise.resolve({ projects, tasks: [], sessions: [], operations: [], selection: { project_id: null, task_id: null, session_id: null } })
    : implementation(command, args));
  expect(await useDevelopment.getState().load('alice')).toBe(true);
  const epoch = useDevelopment.getState().epoch;
  const key = developmentDraftKey('conversation', 'p', 'draft');
  useDevelopmentDrafts.setState({ entries: { [key]: { kind: 'conversation', value: 'keep first-project draft', baseline: '' } } });
  projects = [{ id: 'p', owner: 'host-owner-uuid', name: 'Project', path: '/repo', directory_identity: 'directory', archived: false, created_at: 1, last_opened_at: null }];
  expect(await useDevelopment.getState().load()).toBe(true);
  expect(useDevelopment.getState()).toMatchObject({ epoch, workspaceOwner: 'account:alice' });
  expect(useDesktopPersistence.getState().owner).toBe('account:alice');
  expect(useDevelopmentDrafts.getState().entries[key].value).toBe('keep first-project draft');
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'read_development_desktop_state')).toHaveLength(1);
});

it('does not resurrect a draft explicitly cleared before conflict recovery', async () => {
  await initializeDesktopState('alice');
  const key = developmentDraftKey('conversation', 'p', 'draft', 'message');
  useDevelopmentDrafts.setState({ entries: { [key]: { kind: 'conversation', value: 'sent draft', baseline: '' } } });
  expect(await flushDesktopState()).toBe(true);
  useDevelopmentDrafts.setState({ entries: {} });
  saved = { revision: saved.revision + 1, value: { ...saved.value, remoteOnly: true } };
  expect(await flushDesktopState()).toBe(false);
  await reloadDesktopState();
  expect(useDevelopmentDrafts.getState().entries).toEqual({});
  expect(await flushDesktopState()).toBe(true);
  expect(saved.value.drafts).toMatchObject({ entries: {} });
});

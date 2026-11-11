import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { UnclassifiedPanel } from '../src/components/development/UnclassifiedPanel';
import { UnclassifiedSessionRecovery } from '../src/components/development/UnclassifiedSessionRecovery';
import { hasUnclassifiedRecoveryDraft, preserveClassifiedConversationDraft, readRecoveryTarget, recoveryMessages, recoveryWorkspace } from '../src/components/development/unclassifiedRecovery';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { useDesktopPersistence } from '../src/stores/developmentDesktop';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { chooseContext } from '../src/stores/developmentSelection';
import { useDevelopment, type DevelopmentSession, type DevelopmentTask, type Project } from '../src/stores/developmentStore';
import type { StoredWorkspace, WorkspaceView } from '../src/stores/developmentDelivery';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({
  useLocale: () => ({ locale: 'en-US', t: (key: string, variables?: Record<string, unknown>) => variables?.name ? `${key}:${variables.name}` : key }),
  t: (key: string) => key,
  formatDateTime: (date: Date) => date.toISOString(),
}));

const project: Project = { id: 'project', owner: 'alice', name: 'Original project', path: '/repo', directory_identity: 'repo-identity', archived: false, created_at: 1, last_opened_at: null };
const task: DevelopmentTask = { id: 'task', owner: 'alice', project_id: project.id, goal: 'Existing goal', requirements: '', status: 'todo', created_at: 1 };
const source: DevelopmentSession = { id: 'legacy', owner: 'alice', engine: 'codex', native_id: 'original-native-id', workspace: '/worktree', executable: '/bin/codex', model: null, project_id: null, task_id: null };
const workspace: StoredWorkspace = { id: 'bound-worktree', owner: 'alice', project_id: project.id, path: source.workspace, directory_identity: 'tree-identity', kind: 'imported', common_dir: '/repo/.git', common_identity: 'git-identity', created_operation: null, base_commit: null };
const original = { ...workspace, id: 'original-project', kind: 'original' as const, path: project.path, directory_identity: project.directory_identity };
const workspaceView = (bound = workspace): WorkspaceView => ({ workspaces: [original, bound], selected_workspace_id: bound.id, operation: null, operations: [] });
const snapshot = (events: AgentSession['events'] = []): AgentSession => ({ id: source.id, engine: source.engine, native_id: source.native_id, workspace: source.workspace, generation: 'retained-generation', status: 'closed', history_truncated: true, events, pending: [], error: null });
const callbacks = () => ({ onClose: vi.fn(), onClassified: vi.fn(), onManageProjects: vi.fn(), onManageTasks: vi.fn() });
let classified: DevelopmentSession | null = null;
function readCommand(command: string) {
  if (command === 'read_development_session_summaries') return [{ session_id: source.id, preview: 'Saved preview', last_activity_at: 1_700_000_000 }];
  if (command === 'get_agent_session') return snapshot([{ seq: 1, kind: 'userMessage', text: 'Available original message', data: null }]);
  if (command === 'read_development_workspaces') return workspaceView();
  if (command === 'read_development_workspace') return { projects: [project], tasks: [task], sessions: [classified ?? source], operations: [], selection: { project_id: null, task_id: null, session_id: null } };
  if (command === 'select_development_context') return null;
  if (command === 'read_development_desktop_state' || command === 'save_development_desktop_state') return { revision: 0, value: {} };
  throw new Error(`Unexpected recovery command: ${command}`);
}

beforeEach(() => {
  classified = null;
  useDevelopment.setState({ projects: [project], tasks: [task], sessions: [source], loaded: true, operations: [], workspaceOwner: 'account:alice', selection: { project_id: null, task_id: null, session_id: null }, tabs: [], activeTabId: null, sessionNames: {}, reading: {} });
  useDesktopPersistence.setState({ owner: 'account:alice', loaded: true, loading: false, dirty: false, value: {} });
  invoke.mockImplementation(async (command: string) => {
    if (command === 'classify_agent_session') { classified = { ...source, project_id: project.id, task_id: task.id }; return null; }
    return readCommand(command);
  });
});
afterEach(() => {
  cleanup();
  useDevelopment.getState().reset();
  useAgentRunner.getState().reset();
  useDevelopmentDrafts.getState().clear();
  vi.restoreAllMocks();
  invoke.mockReset();
});

it('opens the recovery browser from unclassified rows without requiring or classifying a current task', () => {
  const onOpen = vi.fn();
  render(<UnclassifiedPanel sessions={[source]} busy={false} onOpen={onOpen} />);
  fireEvent.click(screen.getByTestId(`dev-session-recover-${source.id}`));
  expect(onOpen).toHaveBeenCalledWith(source);
  expect(invoke).not.toHaveBeenCalled();
});

it('reads original metadata, excerpts and retained messages without selecting, running, resuming or authorizing', async () => {
  const events: AgentSession['events'] = [
    { seq: 1, kind: 'userMessage', text: 'Original user message', data: null },
    { seq: 2, kind: 'agentMessage', text: 'Original assistant message', data: null },
    { seq: 3, kind: 'toolResult', text: 'Secret tool payload', data: null },
    { seq: 4, kind: 'reasoning', text: 'Hidden reasoning', data: { type: 'reasoning', content: 'Never show this' } },
  ];
  invoke.mockImplementation(async (command: string) => command === 'get_agent_session' ? snapshot(events) : readCommand(command));
  const props = callbacks();
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  await screen.findByText('Original assistant message');
  expect(screen.getByText(source.native_id!)).toBeTruthy();
  expect(screen.getByText(source.executable!)).toBeTruthy();
  expect(screen.getByText('Saved preview')).toBeTruthy();
  expect(screen.getByText('workbenchNavigation.recoveryNoTaskHistory')).toBeTruthy();
  expect(screen.getByText('agent.truncated')).toBeTruthy();
  expect(screen.queryByText('Secret tool payload')).toBeNull();
  expect(screen.queryByText('Hidden reasoning')).toBeNull();
  expect(invoke.mock.calls.every(([name]) => ['get_agent_session', 'read_development_session_summaries', 'read_development_workspaces'].includes(name))).toBe(true);
  expect(useAgentRunner.getState().selected).toBeNull();
  expect(useDevelopment.getState().selection.session_id).toBeNull();
  expect(props.onClassified).not.toHaveBeenCalled();
});

it('classifies by the task worktree rather than its different project root, then opens the original ID and rekeys its unsent draft', async () => {
  const props = callbacks();
  const from = developmentDraftKey('conversation', null, source.id, 'message');
  const to = developmentDraftKey('conversation', project.id, source.id, 'message');
  const unrelated = developmentDraftKey('conversation', 'another-project', 'draft', 'message');
  useDevelopmentDrafts.setState({ entries: { [from]: { kind: 'conversation', value: 'Continue without losing this', baseline: '' }, [unrelated]: { kind: 'conversation', value: 'Unrelated draft', baseline: '' } } });
  useDevelopment.setState({ sessionNames: { [source.id]: 'Original title' }, reading: { [source.id]: { top: 42, follow: false } } });
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  const radio = await screen.findByRole('radio', { name: 'Original project / Existing goal' });
  expect(radio.hasAttribute('disabled')).toBe(false);
  fireEvent.click(radio);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }));
  await waitFor(() => expect(props.onClassified).toHaveBeenCalledWith({ ...source, project_id: project.id, task_id: task.id }));
  expect(invoke.mock.calls.filter(([name]) => name === 'classify_agent_session')).toEqual([['classify_agent_session', { input: { session_id: source.id, task_id: task.id } }]]);
  expect(invoke.mock.calls.filter(([name]) => name === 'read_development_workspaces').length).toBeGreaterThanOrEqual(2);
  expect(useDevelopmentDrafts.getState().entries[to].value).toBe('Continue without losing this');
  expect(useDevelopmentDrafts.getState().entries[unrelated].value).toBe('Unrelated draft');
  expect(useDevelopment.getState().sessionNames[source.id]).toBe('Original title');
  expect(useDevelopment.getState().reading[source.id]).toEqual({ top: 42, follow: false });
  expect(useDevelopment.getState().selection.session_id).toBeNull();
});

it('does not enable a task just because its project root matches the original session directory', async () => {
  const originalSession = { ...source, workspace: project.path };
  useDevelopment.setState({ sessions: [originalSession] });
  invoke.mockImplementation(async (command: string) => command === 'get_agent_session' ? { ...snapshot(), workspace: originalSession.workspace } : readCommand(command));
  render(<UnclassifiedSessionRecovery session={originalSession} {...callbacks()} />);
  const radio = await screen.findByRole('radio', { name: 'Original project / Existing goal' });
  expect(radio.hasAttribute('disabled')).toBe(true);
  expect(screen.getByText(/workbenchNavigation.recoveryTarget.differentWorkspace/)).toBeTruthy();
  fireEvent.click(radio);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }));
  expect(invoke.mock.calls.some(([name]) => name === 'classify_agent_session')).toBe(false);
});

it('blocks a workspace changed since target selection before classification is dispatched', async () => {
  let reads = 0;
  invoke.mockImplementation(async (command: string) => command === 'read_development_workspaces' ? workspaceView(++reads === 1 ? workspace : { ...workspace, directory_identity: 'replaced-directory' }) : readCommand(command));
  render(<UnclassifiedSessionRecovery session={source} {...callbacks()} />);
  fireEvent.click(await screen.findByRole('radio', { name: 'Original project / Existing goal' }));
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }));
  await screen.findByRole('alert');
  expect(invoke.mock.calls.some(([name]) => name === 'classify_agent_session')).toBe(false);
});

it('does not dispatch classification when a target is archived during its final workspace read', async () => {
  const recheck = Promise.withResolvers<WorkspaceView>();
  let reads = 0;
  invoke.mockImplementation((command: string) => command === 'read_development_workspaces' && ++reads > 1 ? recheck.promise : Promise.resolve(readCommand(command)));
  render(<UnclassifiedSessionRecovery session={source} {...callbacks()} />);
  fireEvent.click(await screen.findByRole('radio'));
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }));
  await act(async () => {
    useDevelopment.setState({ projects: [{ ...project, archived: true }] });
    recheck.resolve(workspaceView());
  });
  await screen.findByRole('alert');
  expect(invoke.mock.calls.some(([name]) => name === 'classify_agent_session')).toBe(false);
});

it.each(['project_busy', 'project_archived', 'workspace_changed'])('preserves the host classification refusal %s and the original draft', async code => {
  const from = developmentDraftKey('conversation', null, source.id, 'message');
  useDevelopmentDrafts.setState({ entries: { [from]: { kind: 'conversation', value: 'Still unsent', baseline: '' } } });
  invoke.mockImplementation(async (command: string) => { if (command === 'classify_agent_session') throw code; return readCommand(command); });
  const props = callbacks();
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  fireEvent.click(await screen.findByRole('radio', { name: 'Original project / Existing goal' }));
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }));
  await screen.findByRole('alert');
  expect(useDevelopmentDrafts.getState().entries[from].value).toBe('Still unsent');
  expect(props.onClassified).not.toHaveBeenCalled();
});

it('provides executable project and task management entries when there is no matching task', async () => {
  useDevelopment.setState({ tasks: [] });
  const props = callbacks();
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  await screen.findByText('workbenchNavigation.recoveryNoTarget');
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryManageProjects' }));
  expect(props.onClose).toHaveBeenCalled();
  expect(props.onManageProjects).toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryManageTasks:Original project' }));
  expect(props.onManageTasks).toHaveBeenCalledWith(project.id);
  expect(invoke.mock.calls.some(([name]) => name === 'save_development_task' || name === 'create_development_project')).toBe(false);
});

it('keeps archived targets disabled and explains that they must be restored', async () => {
  useDevelopment.setState({ projects: [{ ...project, archived: true }] });
  render(<UnclassifiedSessionRecovery session={source} {...callbacks()} />);
  expect((await screen.findByRole('radio')).hasAttribute('disabled')).toBe(true);
  expect(screen.getByText(/workbenchNavigation.recoveryTarget.archived/)).toBeTruthy();
  expect(invoke.mock.calls.some(([name]) => name === 'read_development_workspaces')).toBe(false);
});

it('shows missing history honestly and never probes an unrelated task to fabricate it', async () => {
  invoke.mockImplementation(async (command: string) => { if (command === 'get_agent_session') throw 'legacy_session_read_only'; if (command === 'read_development_session_summaries') return []; return readCommand(command); });
  render(<UnclassifiedSessionRecovery session={source} {...callbacks()} />);
  await screen.findByText('workbenchNavigation.recoveryNoMessages');
  expect(screen.getByText('workbenchNavigation.recoveryNoTaskHistory')).toBeTruthy();
  expect(screen.getByText('workbenchNavigation.noPreview')).toBeTruthy();
  expect(invoke.mock.calls.some(([name]) => name === 'list_development_runs')).toBe(false);
});

it('uses available durable runs only for the record’s original task and session, without reassigning partial ownership', async () => {
  const partial = { ...source, task_id: task.id };
  useDevelopment.setState({ sessions: [partial] });
  const run = { id: 'saved-run', session_id: source.id, task_id: task.id, engine: source.engine, source: 'native_agent', status: 'completed', started_at: 1, finished_at: 2, summary: '', reason: null, fingerprint: '', truncated: true, events: [{ seq: 7, kind: 'agentMessage', text: 'Original durable message', data: null, run: 'saved-run' }] };
  invoke.mockImplementation(async (command: string) => command === 'list_development_runs' ? [run, { ...run, id: 'foreign-run', session_id: 'another-session', events: [{ ...run.events[0], text: 'Unrelated message' }] }] : readCommand(command));
  render(<UnclassifiedSessionRecovery session={partial} {...callbacks()} />);
  await screen.findByText('Original durable message');
  expect(screen.queryByText('Unrelated message')).toBeNull();
  expect(invoke).toHaveBeenCalledWith('list_development_runs', { taskId: task.id });
  expect(screen.getByText('workbenchNavigation.recoveryLinkedBoundary')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }).hasAttribute('disabled')).toBe(true);
});

it('reconciles an unknown classification by the same saved ID without replaying classification', async () => {
  const props = callbacks();
  invoke.mockImplementation(async (command: string) => { if (command === 'classify_agent_session') throw new Error('lost transport response'); return readCommand(command); });
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  fireEvent.click(await screen.findByRole('radio'));
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }));
  await screen.findByText('workbenchNavigation.recoveryUnknownResult');
  classified = { ...source, project_id: project.id, task_id: task.id };
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryInspect' }));
  await waitFor(() => expect(props.onClassified).toHaveBeenCalledWith(classified));
  expect(invoke.mock.calls.filter(([name]) => name === 'classify_agent_session')).toHaveLength(1);
  expect(invoke).toHaveBeenCalledWith('read_development_workspace', {});
});

it.each(['account', 'context'] as const)('ignores late history and target results after an %s switch', async kind => {
  const history = Promise.withResolvers<AgentSession>();
  const workspaces = Promise.withResolvers<WorkspaceView>();
  invoke.mockImplementation((command: string) => command === 'get_agent_session' ? history.promise : command === 'read_development_workspaces' ? workspaces.promise : Promise.resolve(readCommand(command)));
  const props = callbacks();
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  await act(async () => {
    if (kind === 'account') useDevelopment.getState().reset();
    else await chooseContext({ project_id: project.id, task_id: task.id, session_id: null });
  });
  await act(async () => { history.resolve(snapshot([{ seq: 8, kind: 'agentMessage', text: 'Late private history', data: null }])); workspaces.resolve(workspaceView()); });
  expect(screen.queryByTestId('unclassified-session-recovery')).toBeNull();
  expect(screen.queryByText('Late private history')).toBeNull();
  expect(props.onClose).toHaveBeenCalled();
  expect(props.onClassified).not.toHaveBeenCalled();
});

it('does not reveal a late classification after a context switch but keeps its confirmed draft reachable', async () => {
  const classification = Promise.withResolvers<void>();
  invoke.mockImplementation((command: string) => command === 'classify_agent_session' ? classification.promise : Promise.resolve(readCommand(command)));
  const from = developmentDraftKey('conversation', null, source.id, 'message');
  useDevelopmentDrafts.setState({ entries: { [from]: { kind: 'conversation', value: 'Retained during switch', baseline: '' } } });
  const props = callbacks();
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  fireEvent.click(await screen.findByRole('radio'));
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }));
  await waitFor(() => expect(invoke.mock.calls.some(([name]) => name === 'classify_agent_session')).toBe(true));
  await act(async () => { await chooseContext({ project_id: project.id, task_id: task.id, session_id: null }); classified = { ...source, project_id: project.id, task_id: task.id }; classification.resolve(); });
  expect(props.onClassified).not.toHaveBeenCalled();
  expect(useDevelopment.getState().selection).toEqual({ project_id: project.id, task_id: task.id, session_id: null });
  const to = developmentDraftKey('conversation', project.id, source.id, 'message');
  expect(useDevelopmentDrafts.getState().entries[to].value).toBe('Retained during switch');
});

it('never rekeys a new account’s drafts or opens its same-ID session after a late classification', async () => {
  const classification = Promise.withResolvers<void>();
  invoke.mockImplementation((command: string) => command === 'classify_agent_session' ? classification.promise : Promise.resolve(readCommand(command)));
  const props = callbacks();
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  fireEvent.click(await screen.findByRole('radio'));
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }));
  await waitFor(() => expect(invoke.mock.calls.some(([name]) => name === 'classify_agent_session')).toBe(true));
  let newDraftKey = '';
  await act(async () => {
    useDevelopment.getState().reset();
    useDevelopment.setState({ workspaceOwner: 'account:bob', projects: [{ ...project, owner: 'bob' }], tasks: [{ ...task, owner: 'bob' }], sessions: [{ ...source, owner: 'bob', project_id: project.id, task_id: task.id }] });
    newDraftKey = developmentDraftKey('conversation', project.id, source.id, 'message');
    useDevelopmentDrafts.setState({ entries: { [newDraftKey]: { kind: 'conversation', value: 'New account draft', baseline: '' } } });
    classification.resolve();
  });
  expect(props.onClassified).not.toHaveBeenCalled();
  expect(useDevelopmentDrafts.getState().entries[newDraftKey].value).toBe('New account draft');
  expect(useDevelopment.getState().workspaceOwner).toBe('account:bob');
});

it('provides the real project creation entry when the account has no projects or tasks', async () => {
  useDevelopment.setState({ projects: [], tasks: [] });
  const props = callbacks();
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  await screen.findByText('workbenchNavigation.recoveryNoTarget');
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryManageProjects' }));
  expect(props.onManageProjects).toHaveBeenCalledOnce();
  expect(screen.queryByRole('radio')).toBeNull();
  expect(invoke.mock.calls.some(([name]) => name === 'classify_agent_session')).toBe(false);
});

it('disables a target whose actual task workspace cannot be read', async () => {
  invoke.mockImplementation(async (command: string) => { if (command === 'read_development_workspaces') throw 'workspace_changed'; return readCommand(command); });
  render(<UnclassifiedSessionRecovery session={source} {...callbacks()} />);
  expect((await screen.findByRole('radio')).hasAttribute('disabled')).toBe(true);
  expect(screen.getByText(/workbenchNavigation.recoveryTarget.workspaceUnavailable/)).toBeTruthy();
  expect(screen.getByText(/workbenchNavigation.recoveryUnknownWorkspace/)).toBeTruthy();
  expect(invoke.mock.calls.some(([name]) => name === 'classify_agent_session')).toBe(false);
});

it('never falls back to the project original when the selected workspace ID is missing', () => {
  expect(recoveryWorkspace({ ...workspaceView(), selected_workspace_id: 'missing-workspace' })).toBeNull();
  expect(recoveryWorkspace({ ...workspaceView(), selected_workspace_id: null })).toEqual(original);
});

it('excludes cross-account targets before any workspace read', async () => {
  const target = await readRecoveryTarget(source, { ...project, owner: 'bob' }, task);
  expect(target.reason).toBe('workspaceUnavailable');
  expect(invoke).not.toHaveBeenCalled();
});

it('preserves colliding draft indexes verbatim and exposes them to the parent recovery entry', () => {
  const from = developmentDraftKey('conversation', null, source.id, 'message');
  const to = developmentDraftKey('conversation', project.id, source.id, 'message');
  const skills = developmentDraftKey('conversation', null, source.id, 'skills');
  useDevelopmentDrafts.setState({ entries: { [from]: { kind: 'conversation', value: 'Original draft', baseline: '' }, [to]: { kind: 'conversation', value: 'Later target draft', baseline: '' }, [skills]: { kind: 'conversation', value: ['keep-index'], baseline: [] } } });
  preserveClassifiedConversationDraft(source, { ...source, project_id: project.id, task_id: task.id });
  expect(useDevelopmentDrafts.getState().entries[to].value).toBe('Later target draft');
  expect(useDevelopmentDrafts.getState().entries[from].value).toBe('Original draft');
  expect(hasUnclassifiedRecoveryDraft({ ...source, project_id: project.id, task_id: task.id })).toBe(true);
  expect(useDevelopmentDrafts.getState().entries[skills].value).toEqual(['keep-index']);
});

it('shows both drafts after classification without overwriting the target, then swaps only on an explicit choice', async () => {
  const props = callbacks();
  const from = developmentDraftKey('conversation', null, source.id, 'message');
  const to = developmentDraftKey('conversation', project.id, source.id, 'message');
  useDevelopmentDrafts.setState({ entries: { [from]: { kind: 'conversation', value: 'Original unsent draft', baseline: '' }, [to]: { kind: 'conversation', value: 'Existing current draft', baseline: '' } } });
  render(<UnclassifiedSessionRecovery session={source} {...props} />);
  fireEvent.click(await screen.findByRole('radio'));
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryClassify' }));
  await screen.findByLabelText('workbenchNavigation.recoveryDraftOriginal');
  expect(props.onClassified).not.toHaveBeenCalled();
  expect(screen.getByLabelText('workbenchNavigation.recoveryDraftOriginal')).toHaveProperty('value', 'Original unsent draft');
  expect(screen.getByLabelText('workbenchNavigation.recoveryDraftCurrent')).toHaveProperty('value', 'Existing current draft');
  expect(useDevelopmentDrafts.getState().entries[to].value).toBe('Existing current draft');
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryUseOriginal' }));
  expect(useDevelopmentDrafts.getState().entries[to].value).toBe('Original unsent draft');
  expect(useDevelopmentDrafts.getState().entries[from].value).toBe('Existing current draft');
  expect(props.onClassified).toHaveBeenCalledWith(classified);
  expect(hasUnclassifiedRecoveryDraft(classified!)).toBe(true);
  expect(invoke.mock.calls.filter(([name]) => name === 'classify_agent_session')).toHaveLength(1);
});

it('opens an already-classified session’s current draft without deleting the recoverable draft or reclassifying', async () => {
  const saved = { ...source, project_id: project.id, task_id: task.id };
  useDevelopment.setState({ sessions: [saved] });
  const from = developmentDraftKey('conversation', null, source.id, 'message');
  const to = developmentDraftKey('conversation', project.id, source.id, 'message');
  useDevelopmentDrafts.setState({ entries: { [from]: { kind: 'conversation', value: 'Still recoverable', baseline: '' }, [to]: { kind: 'conversation', value: 'Keep current', baseline: '' } } });
  const props = callbacks();
  render(<UnclassifiedSessionRecovery session={saved} {...props} />);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryOpenCurrent' }));
  expect(props.onClassified).toHaveBeenCalledWith(saved);
  expect(useDevelopmentDrafts.getState().entries[from].value).toBe('Still recoverable');
  expect(useDevelopmentDrafts.getState().entries[to].value).toBe('Keep current');
  expect(invoke.mock.calls.some(([name]) => name === 'classify_agent_session')).toBe(false);
});

it('copies the recoverable draft while preserving the current draft, then opens without dispatching any execution', async () => {
  const saved = { ...source, project_id: project.id, task_id: task.id };
  useDevelopment.setState({ sessions: [saved] });
  const from = developmentDraftKey('conversation', null, source.id, 'message');
  const to = developmentDraftKey('conversation', project.id, source.id, 'message');
  useDevelopmentDrafts.setState({ entries: { [from]: { kind: 'conversation', value: 'Copy this original', baseline: '' }, [to]: { kind: 'conversation', value: 'Do not replace this', baseline: '' } } });
  const clipboard = vi.fn().mockResolvedValue(undefined);
  const descriptor = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: clipboard } });
  try {
    const props = callbacks();
    render(<UnclassifiedSessionRecovery session={saved} {...props} />);
    fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.recoveryCopyOriginal' }));
    await waitFor(() => expect(props.onClassified).toHaveBeenCalledWith(saved));
    expect(clipboard).toHaveBeenCalledWith('Copy this original');
    expect(useDevelopmentDrafts.getState().entries[from].value).toBe('Copy this original');
    expect(useDevelopmentDrafts.getState().entries[to].value).toBe('Do not replace this');
    expect(invoke.mock.calls.some(([name]) => ['classify_agent_session', 'open_agent_session', 'send_agent_message', 'set_development_workspace_trust'].includes(name))).toBe(false);
  } finally {
    if (descriptor) Object.defineProperty(navigator, 'clipboard', descriptor);
    else Reflect.deleteProperty(navigator, 'clipboard');
  }
});

it('does not use tool or reasoning blocks as message-text fallback', () => {
  expect(recoveryMessages([
    { seq: 1, kind: 'message', text: 'Raw hidden payload', data: { type: 'assistant', content: [{ type: 'thinking', thinking: 'secret' }, { type: 'tool_use', input: 'private' }] } },
    { seq: 2, kind: 'message', text: 'Raw payload', data: { type: 'assistant', content: [{ type: 'text', text: 'Visible saved answer' }, { type: 'tool_use', input: 'private' }] } },
  ])).toEqual([{ seq: 2, role: 'assistant', text: 'Visible saved answer' }]);
});

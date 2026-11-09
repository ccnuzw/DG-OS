import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ProjectSessionTree } from '../src/components/development/ProjectSessionTree';
import { useDevelopment, type DevelopmentSession, type Project } from '../src/stores/developmentStore';
import { useAgentRunner } from '../src/stores/agentRunnerStore';

const { invoke, picker } = vi.hoisted(() => ({ invoke: vi.fn(), picker: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: picker }));
vi.mock('@tauri-apps/api/path', () => ({ basename: async (path: string) => path.split('/').pop(), join: async (...parts: string[]) => parts.join('/') }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string, variables?: Record<string, unknown>) => variables?.count === undefined ? key : `${key}:${variables.count}` }), t: (key: string) => key, formatDateTime: (date: Date) => date.toISOString() }));

const project: Project = { id: 'p', owner: 'alice', name: 'Workbench', path: '/workbench', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: 1 };
const sessions: DevelopmentSession[] = Array.from({ length: 8 }, (_, index) => ({ id: `s${index}`, owner: 'alice', engine: 'codex', native_id: null, workspace: '/workbench', executable: null, model: null, project_id: 'p', task_id: null }));
const callbacks = { onSelect: vi.fn(), onNewConversation: vi.fn(), onSelectProject: vi.fn(), onSelectDraft: vi.fn(), onManageProject: vi.fn() };

beforeEach(() => {
  vi.useFakeTimers();
  useDevelopment.setState({ projects: [project], sessions, tasks: [], operations: [], selection: { project_id: 'p', task_id: null, session_id: 's7' }, sessionNames: Object.fromEntries(sessions.map((session, index) => [session.id, `Conversation ${index}`])), tabs: [], activeTabId: null, workspaceOwner: 'alice' });
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_session_summaries') return sessions.map((session, index) => ({ session_id: session.id, preview: `Message preview ${index}`, last_activity_at: 1_700_000_000 - index * 60 }));
    throw new Error(`Unexpected navigation command: ${command}`);
  });
});
afterEach(() => {
  cleanup();
  useDevelopment.getState().reset();
  useAgentRunner.getState().reset();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.clearAllMocks();
  invoke.mockReset();
});

it('uses one metadata batch for real previews and times, five recent rows plus the current row', async () => {
  render(<ProjectSessionTree busy={false} {...callbacks} />);
  await act(async () => { await vi.advanceTimersByTimeAsync(210); });
  expect(invoke.mock.calls).toEqual([['read_development_session_summaries', {}]]);
  expect(screen.getAllByTestId(/^dev-session-item-/)).toHaveLength(6);
  expect(screen.getByTestId('dev-session-item-s7').getAttribute('aria-pressed')).toBe('true');
  expect(screen.queryByTestId('dev-session-item-s5')).toBeNull();
  expect(screen.getByText('Message preview 0')).toBeTruthy();
  expect(screen.getByTestId('dev-session-item-s0').querySelector('time')?.dateTime).toBe(new Date(1_700_000_000 * 1000).toISOString());
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.showMoreSessions:2' }));
  expect(screen.getAllByTestId(/^dev-session-item-/)).toHaveLength(8);
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(callbacks.onSelect).not.toHaveBeenCalled();
  expect(screen.queryByTestId('dev-project-detail')).toBeNull();
});

it('collapses the selected project without changing selection or executing anything', async () => {
  render(<ProjectSessionTree busy={false} {...callbacks} />);
  await act(async () => { await vi.advanceTimersByTimeAsync(210); });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.collapseProject: Workbench' }));
  expect(screen.queryByTestId('dev-session-item-s7')).toBeNull();
  expect(useDevelopment.getState().selection.session_id).toBe('s7');
  expect(Object.values(callbacks).every(callback => callback.mock.calls.length === 0)).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.expandProject: Workbench' }));
  expect(screen.getByTestId('dev-session-item-s7')).toBeTruthy();
  expect(invoke).toHaveBeenCalledTimes(1);
});

it('searches metadata beyond the recent limit and retains the current conversation', async () => {
  render(<ProjectSessionTree busy={false} {...callbacks} />);
  await act(async () => { await vi.advanceTimersByTimeAsync(210); });
  fireEvent.change(screen.getByTestId('dev-unified-search'), { target: { value: 'Message preview 6' } });
  expect(screen.getByTestId('dev-session-item-s6')).toBeTruthy();
  expect(screen.getByTestId('dev-session-item-s7')).toBeTruthy();
  expect(screen.queryByTestId('dev-session-item-s0')).toBeNull();
  expect(screen.queryByRole('button', { name: /showMoreSessions/ })).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(1);
});

it('routes each draft and project action to its exact selection callback without runtime commands', () => {
  const first = { id: 'draft1', projectId: 'p', taskId: null, sessionId: null, title: 'First draft' };
  const second = { ...first, id: 'draft2', title: 'Second draft' };
  useDevelopment.setState({ tabs: [first, second], activeTabId: second.id, selection: { project_id: 'p', task_id: null, session_id: null } });
  render(<ProjectSessionTree busy={false} {...callbacks} />);
  expect(screen.getByTestId('dev-draft-item-draft2').getAttribute('aria-pressed')).toBe('true');
  fireEvent.click(screen.getByTestId('dev-draft-item-draft1'));
  expect(callbacks.onSelectDraft).toHaveBeenCalledWith(first);
  fireEvent.click(screen.getByTestId('dev-project-item-p'));
  expect(callbacks.onSelectProject).toHaveBeenCalledWith('p');
  fireEvent.click(screen.getByRole('button', { name: 'dev.newSession: Workbench' }));
  expect(callbacks.onNewConversation).toHaveBeenCalledWith('p');
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.manageProject: Workbench' }));
  expect(callbacks.onManageProject).toHaveBeenCalledWith('p');
  fireEvent.click(screen.getByTestId('dev-session-item-s0'));
  expect(callbacks.onSelect).toHaveBeenCalledWith(sessions[0]);
  expect(invoke).not.toHaveBeenCalled();
});

it('retains archived and unclassified history on demand, without inspecting projects', async () => {
  useDevelopment.setState({ projects: [project, { ...project, id: 'archive', name: 'Archive', archived: true }], sessions: [...sessions, { ...sessions[0], id: 'old', project_id: 'archive' }, { ...sessions[0], id: 'loose', project_id: null }] });
  render(<ProjectSessionTree busy={false} {...callbacks} />);
  expect(screen.queryByRole('button', { name: 'Archive' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /dev.archived/ }));
  expect(screen.getByRole('button', { name: 'Archive' })).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.expandProject: Archive' }));
  expect(screen.getByTestId('dev-session-item-old')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'dev.newSession: Archive' }).hasAttribute('disabled')).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: /dev.unclassified/ }));
  expect(screen.getByTestId('dev-session-item-loose')).toBeTruthy();
  await act(async () => { await vi.advanceTimersByTimeAsync(210); });
  expect(invoke.mock.calls.every(([command]) => command === 'read_development_session_summaries')).toBe(true);
});

it('drops a metadata response after the account epoch changes', async () => {
  const pending = Promise.withResolvers<unknown>();
  invoke.mockReturnValue(pending.promise);
  render(<ProjectSessionTree busy={false} {...callbacks} />);
  await act(async () => { await vi.advanceTimersByTimeAsync(210); });
  act(() => { useDevelopment.getState().reset(); });
  await act(async () => { pending.resolve([{ session_id: 's0', preview: 'Old account secret', last_activity_at: 1_700_000_000 }]); });
  expect(screen.queryByText('Old account secret')).toBeNull();
});

it('keeps presentation controls available while context selection is busy', () => {
  render(<ProjectSessionTree busy {...callbacks} />);
  expect(screen.getByTestId('dev-project-item-p').hasAttribute('disabled')).toBe(true);
  expect(screen.getByTestId('dev-session-item-s7').hasAttribute('disabled')).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchNavigation.collapseProject: Workbench' }));
  expect(screen.getByRole('button', { name: 'workbenchNavigation.expandProject: Workbench' }).hasAttribute('disabled')).toBe(false);
});

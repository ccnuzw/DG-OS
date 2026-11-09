import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { ConversationHandoff } from '../src/components/development/ConversationHandoff';
import { useDevelopment, type Project, type DevelopmentSession } from '../src/stores/developmentStore';
import { useDevelopmentDrafts, developmentDraftKey } from '../src/stores/developmentDrafts';
import type { StoredHandoff } from '../src/stores/developmentDelivery';
const { invoke, surface } = vi.hoisted(() => ({ invoke: vi.fn(), surface: { active: true } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));
vi.mock('../src/workspace/SurfaceContext', () => ({ useSurfaceActive: () => surface.active }));
const project: Project = { id: 'p', owner: 'owner', name: 'Project', path: '/fixture', directory_identity: 'directory', archived: false, created_at: 1, last_opened_at: null };
const source: DevelopmentSession = { id: 'source', owner: 'owner', project_id: 'p', task_id: 'task', engine: 'codex', native_id: 'native', workspace: '/fixture', executable: null, model: 'old-model' };
const configuration = { engine: 'claude', provider: { provider_id: 'provider', model_id: 'new-model', provider_revision: 1 } };
const handoff: StoredHandoff = { id: 'handoff', owner: 'owner', project_id: 'p', task_id: 'task', source_session_id: 'source', target_engine: 'claude', permission_epoch: 1, summary: 'Observed context', fingerprint: 'fp', target_session_id: null, created_at: 1 };
function fixture() {
  useDevelopment.setState({ projects: [project], sessions: [source], selection: { project_id: 'p', task_id: 'task', session_id: 'source' }, loaded: true });
  const state = { rows: [] as StoredHandoff[], acceptance: Promise.withResolvers<unknown>() };
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_cli_provider') return { configuration, revision: 1, permission_epoch: 1, created_at: 1 };
    if (command === 'read_development_cli_provider_catalog') return { project_id: 'p', engine: 'claude', providers: [{ id: 'provider', name: 'Provider', config_revision: 1, enabled: true, availability: { available: true, reason: null }, models: [{ id: 'new-model', name: 'New model', model_type: 'text' }] }] };
    if (command === 'get_development_session_configuration') return { format: 'provider_bound', snapshot: { engine: 'codex', provider_id: 'old', model_id: 'old-model', provider_revision: 1 } };
    if (command === 'read_development_workspaces') return { workspaces: [{ id: 'w', project_id: 'p', path: '/fixture', directory_identity: 'directory', kind: 'original' }], selected_workspace_id: 'w', operations: [], operation: null };
    if (command === 'list_development_handoffs') return structuredClone(state.rows);
    if (command === 'prepare_development_handoff') { state.rows = [structuredClone(handoff)]; return structuredClone(handoff); }
    if (command === 'accept_development_handoff') return state.acceptance.promise;
    throw new Error(`Unexpected handoff command: ${command}`);
  });
  return state;
}
const count = () => invoke.mock.calls.filter(([command]) => command === 'accept_development_handoff').length;
const tree = (ready = vi.fn(async () => {}), session = source) => <ConversationHandoff project={project} source={session} onReady={ready} onCancel={() => {}} />;
async function prepare() {
  fireEvent.change(screen.getByLabelText('conversationHandoff.remaining'), { target: { value: 'Finish the pending work' } });
  await waitFor(() => expect(screen.getByRole('button', { name: 'conversationHandoff.prepare' })).toHaveProperty('disabled', false));
  fireEvent.click(screen.getByRole('button', { name: 'conversationHandoff.prepare' }));
  await screen.findByLabelText('conversationHandoff.summary');
  fireEvent.change(screen.getByLabelText('conversationHandoff.summary'), { target: { value: 'Reviewed editable context' } });
  fireEvent.click(screen.getByRole('checkbox', { name: 'conversationHandoff.trust' }));
  fireEvent.click(screen.getByRole('button', { name: 'conversationHandoff.create' }));
  await waitFor(() => expect(count()).toBe(1));
}
afterEach(() => { cleanup(); useDevelopment.getState().reset(); invoke.mockReset(); surface.active = true; localStorage.clear(); });
it('keeps an unknown acceptance reserved through remount and absent-target recheck without replaying', async () => {
  const state = fixture(); const ready = vi.fn(async () => {}); const view = render(tree(ready)); await prepare();
  view.unmount(); render(tree(ready));
  await act(async () => state.acceptance.reject('transport lost'));
  await screen.findByText('conversationHandoff.reserved');
  fireEvent.click(screen.getByRole('button', { name: 'conversationHandoff.recheck' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'conversationHandoff.recheck' })).toHaveProperty('disabled', false));
  expect(screen.getByRole('button', { name: 'conversationHandoff.create' })).toHaveProperty('disabled', true);
  expect(count()).toBe(1); expect(ready).not.toHaveBeenCalled();
});
it('retains a late accepted target and edited prompt without stealing focus or sending', async () => {
  const state = fixture(); const ready = vi.fn(async () => {}); const view = render(tree(ready)); await prepare(); view.unmount();
  const accepted = { ...handoff, target_session_id: 'target' }; state.rows = [accepted];
  await act(async () => state.acceptance.resolve({ handoff: accepted, session: { id: 'target' } }));
  expect(ready).not.toHaveBeenCalled();
  expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', 'p', 'target', 'message')].value).toBe('Reviewed editable context');
  render(tree(ready)); fireEvent.click(await screen.findByRole('button', { name: 'conversationHandoff.attach' }));
  await waitFor(() => expect(ready).toHaveBeenCalledWith('target'));
  expect(count()).toBe(1);
  expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message')).toBe(false);
});
it('drops a late accepted prompt after account reset', async () => {
  const state = fixture(); const ready = vi.fn(async () => {}); const view = render(tree(ready)); await prepare(); view.unmount();
  useDevelopment.getState().reset();
  await act(async () => state.acceptance.resolve({ handoff: { ...handoff, target_session_id: 'target' }, session: { id: 'target' } }));
  expect(useDevelopmentDrafts.getState().entries).toEqual({}); expect(ready).not.toHaveBeenCalled();
});

it('retains an accepted target as draft-only while inactive and requires explicit attach on return', async () => {
  const state = fixture(); const ready = vi.fn(async () => {});
  const view = render(tree(ready)); await prepare();
  surface.active = false; view.rerender(tree(ready));
  const accepted = { ...handoff, target_session_id: 'target' }; state.rows = [accepted];
  await act(async () => state.acceptance.resolve({ handoff: accepted, session: { id: 'target' } }));
  expect(ready).not.toHaveBeenCalled();
  expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('conversation', 'p', 'target', 'message')].value).toBe('Reviewed editable context');
  fireEvent.click(screen.getByRole('button', { name: 'conversationHandoff.attach' }));
  expect(ready).not.toHaveBeenCalled();
  surface.active = true; view.rerender(tree(ready));
  fireEvent.click(screen.getByRole('button', { name: 'conversationHandoff.attach' }));
  await waitFor(() => expect(ready).toHaveBeenCalledWith('target'));
  expect(count()).toBe(1);
  expect(invoke.mock.calls.some(([command]) => command === 'send_agent_message' || command === 'respond_agent_request')).toBe(false);
});

it('cancels inactive acceptance preflight without losing the draft or leaving the composer busy', async () => {
  const state = fixture(); const ready = vi.fn(async () => {}); const view = render(tree(ready));
  fireEvent.change(screen.getByLabelText('conversationHandoff.remaining'), { target: { value: 'Keep the remaining intent' } });
  await waitFor(() => expect(screen.getByRole('button', { name: 'conversationHandoff.prepare' })).toHaveProperty('disabled', false));
  fireEvent.click(screen.getByRole('button', { name: 'conversationHandoff.prepare' }));
  await screen.findByLabelText('conversationHandoff.summary');
  fireEvent.click(screen.getByRole('checkbox', { name: 'conversationHandoff.trust' }));
  const workspace = Promise.withResolvers<unknown>();
  const fallback = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => command === 'read_development_workspaces' ? workspace.promise : fallback(command, args));
  fireEvent.click(screen.getByRole('button', { name: 'conversationHandoff.create' }));
  surface.active = false; view.rerender(tree(ready));
  await act(async () => workspace.resolve({ workspaces: [{ id: 'w', project_id: 'p', path: '/fixture', directory_identity: 'directory', kind: 'original' }], selected_workspace_id: 'w', operations: [], operation: null }));
  expect(count()).toBe(0);
  expect(ready).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'conversationHandoff.create' }));
  expect(count()).toBe(0);
  expect(screen.getByLabelText<HTMLTextAreaElement>('conversationHandoff.remaining').value).toBe('Keep the remaining intent');
  surface.active = true; view.rerender(tree(ready));
  await waitFor(() => expect(screen.getByRole('button', { name: 'conversationHandoff.create' })).toHaveProperty('disabled', false));
  fireEvent.click(screen.getByRole('button', { name: 'conversationHandoff.create' }));
  await waitFor(() => expect(count()).toBe(1));
  await act(async () => state.acceptance.resolve({ handoff: { ...handoff, target_session_id: 'target' }, session: { id: 'target' } }));
});

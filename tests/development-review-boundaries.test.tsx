import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, expect, it, vi } from 'vitest';
import { ProjectReview } from '../src/components/development/ProjectReview';
import { AgentConversation, Request } from '../src/pages/workbench/AgentConversation';
import { useDevelopment } from '../src/stores/developmentStore';
import { chooseContext } from '../src/stores/developmentSelection';
import { ProjectCapabilities } from '../src/components/development/ProjectCapabilities';
import { useAgentRunner } from '../src/stores/agentRunnerStore';
import { DeliveryPanel } from '../src/components/development/DeliveryPanel';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); invoke.mockReset(); vi.useRealTimers(); });

it('does not reveal a late source response after selecting a different task', async () => {
 const source = Promise.withResolvers<unknown>();
 invoke.mockImplementation((cmd: string) => {
  if (cmd === 'list_development_files') return Promise.resolve({ entries: [{ path: 'old.ts', name: 'old.ts', kind: 'file', size: 10, ignored: false }], truncated: false });
  if (cmd === 'read_development_file') return source.promise;
  return Promise.resolve(null);
 });
 render(<ProjectReview projectId="p" taskId="t" />);
 fireEvent.click(await screen.findByTestId('dev-file-item-old.ts'));
 await waitFor(() => expect(invoke.mock.calls.some(([cmd]) => cmd === 'read_development_file')).toBe(true));
 await act(async () => { await chooseContext({ project_id: 'p2', task_id: 't2', session_id: null }); source.resolve({ path: 'old.ts', text: 'PRIVATE_OLD_SOURCE', size: 10, reason: null, hash: 'hash' }); });
 expect(document.body.textContent).not.toContain('PRIVATE_OLD_SOURCE');
});

it('retains multiple selected answers under the native question id and hides unavailable approvals', async () => {
 const submit = vi.fn(async () => true);
 const { unmount } = render(<Request engine="codex" pending={{ id: 'q1', kind: 'tool/requestUserInput', title: 'Question', data: { questions: [{ id: 'native-question', question: 'Which?', multiSelect: true, options: [{ label: 'First' }, { label: 'Second' }] }] } }} submit={submit} />);
 fireEvent.click(screen.getByRole('button', { name: 'First' })); fireEvent.click(screen.getByRole('button', { name: 'Second' }));
 fireEvent.click(screen.getByRole('button', { name: 'agent.answer' }));
 await waitFor(() => expect(submit).toHaveBeenCalledWith({ answers: { 'native-question': { answers: ['First', 'Second'] } } }));
 unmount();
 render(<Request engine="codex" pending={{ id: 'a1', kind: 'item/commandExecution/requestApproval', title: 'Approve', data: { availableDecisions: ['decline'] } }} submit={submit} />);
 expect(screen.queryByRole('button', { name: 'agent.allow' })).toBeNull();
 fireEvent.click(screen.getByRole('button', { name: 'agent.deny' }));
 await waitFor(() => expect(submit).toHaveBeenLastCalledWith({ decision: 'decline' }));
});

it('drops an old account capability catalog instead of offering its providers', async () => {
 const catalog = Promise.withResolvers<unknown>();
 let reads = 0;
 invoke.mockImplementation((cmd: string) => {
  if (cmd === 'read_development_cli_provider_catalog') return ++reads === 1 ? catalog.promise : Promise.resolve({ project_id: 'p', engine: 'codex', providers: [] });
  return Promise.resolve(null);
 });
 render(<MemoryRouter><ProjectCapabilities projectId="p" /></MemoryRouter>);
 await act(async () => { useDevelopment.getState().reset(); });
 await act(async () => { catalog.resolve({ project_id: 'p', engine: 'codex', providers: [{ id: 'private-provider', name: 'PRIVATE_OLD_PROVIDER', config_revision: 1, enabled: true, availability: { available: true, reason: null }, models: [{ id: 'm', name: 'm', model_type: 'chat' }] }] }); });
 expect(document.body.textContent).not.toContain('PRIVATE_OLD_PROVIDER');
});

it('submits one exact native permission decision without granting unrelated host capabilities', async () => {
 const response = Promise.withResolvers<boolean>();
 const submit = vi.fn(() => response.promise);
 const permissions = { fileSystem: { write: ['/project/output'] }, network: null };
 render(<Request engine="codex" pending={{ id: 'permission-request', kind: 'item/permissions/requestApproval', title: 'Permission', data: { threadId: 'native-thread', turnId: 'turn', itemId: 'item', permissions } }} submit={submit} />);
 const allow = screen.getByRole('button', { name: 'agent.allowTurn' });
 fireEvent.click(allow); fireEvent.click(allow);
 expect(submit).toHaveBeenCalledTimes(1);
 expect(submit).toHaveBeenCalledWith({ permissions, scope: 'turn' });
 expect(invoke).not.toHaveBeenCalled();
 await act(async () => { response.resolve(true); });
 expect(allow.hasAttribute('disabled')).toBe(true);
});

it('keeps the file selected when checking it again after deselection', async () => {
 invoke.mockImplementation((cmd: string) => {
  if (cmd === 'read_development_workspaces') return Promise.resolve({ workspaces: [], selected_workspace_id: null });
  if (cmd === 'read_development_git') return Promise.resolve({ kind: 'git', files: [{ path: 'code.txt', index: ' ', worktree: 'M' }], conflicts: [], initial_staged_paths: [], file_versions: {}, baseline_version: 'baseline' });
  if (cmd === 'list_development_handoffs') return Promise.resolve([]);
  return Promise.resolve(null);
 });
 render(<DeliveryPanel projectId="p" taskId="t" sessions={[]} live={{}} />);
 const file = await screen.findByRole('checkbox', { name: /code.txt/ });
 const stage = screen.getByRole('button', { name: 'delivery.stage' });
 fireEvent.click(file);
 expect((stage as HTMLButtonElement).disabled).toBe(false);
 fireEvent.click(file);
 expect((stage as HTMLButtonElement).disabled).toBe(true);
 fireEvent.click(file);
 expect((file as HTMLInputElement).checked).toBe(true);
 expect((stage as HTMLButtonElement).disabled).toBe(false);
});

it('expires native approvals on session closure and rejects a retained button callback', async () => {
 const pending = { id: 'approval', kind: 'item/commandExecution/requestApproval', title: 'Bash', data: { command: 'write output', availableDecisions: ['accept', 'decline'] } };
 const session = { id: 's', generation: 'generation-s', engine: 'codex' as const, native_id: 'native', workspace: '/tmp/project', status: 'waiting', events: [], pending: [pending], error: null };
 invoke.mockImplementation(async (command: string) => {
  if (command === 'get_development_session_configuration') return { format: 'provider_bound', read_only: false, snapshot: { binding_id: 'binding-s', engine: 'codex', provider_id: 'provider', model_id: 'model', provider_revision: 1, revision: 1, permission_epoch: 0 } };
  throw new Error(`Unexpected command: ${command}`);
 });
 useAgentRunner.setState({ selected: 's', session });
 const view = render(<AgentConversation session={session} drafts={{}} setDraft={() => {}} busy={false} error={false} />);
 const allow = screen.getByRole('button', { name: 'agent.allow' });
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('get_development_session_configuration', { sessionId: 's' }));
 act(() => useAgentRunner.setState({ session: { ...session, status: 'closed', pending: [] } }));
 fireEvent.click(allow);
 view.rerender(<AgentConversation session={{ ...session, status: 'closed', pending: [] }} drafts={{}} setDraft={() => {}} busy={false} error={false} />);
 expect(screen.queryByRole('button', { name: 'agent.allow' })).toBeNull();
 expect(invoke.mock.calls.filter(([command]) => command === 'respond_agent_request')).toHaveLength(0);
});

it('removes a consumed native request after revocation without restoring approval authority', async () => {
 const pending = { id: 'native-approval', kind: 'item/fileChange/requestApproval', title: 'Write', data: { changes: [{ path: '/project/result' }], availableDecisions: ['accept', 'decline'] } };
 const session = { id: 's', generation: 'generation-s', engine: 'codex' as const, native_id: 'native', workspace: '/tmp/project', status: 'waiting', events: [], pending: [pending], error: null };
 invoke.mockImplementation(async (command: string) => {
  if (command === 'respond_agent_request') throw 'capability_revoked';
  if (command === 'get_agent_session') return { ...session, pending: [] };
  return null;
 });
 useAgentRunner.setState({ selected: 's', session });
 function Conversation() {
  const current = useAgentRunner(state => state.session);
  return current && <AgentConversation session={current} drafts={{}} setDraft={() => {}} busy={false} error={false} />;
 }
 render(<Conversation />);
 fireEvent.click(screen.getByRole('button', { name: 'agent.allow' }));
 await waitFor(() => expect(screen.queryByRole('button', { name: 'agent.allow' })).toBeNull());
 expect(useAgentRunner.getState().error).toBe('capability_revoked');
 expect(invoke.mock.calls.filter(([command]) => command === 'respond_agent_request')).toEqual([['respond_agent_request', { input: { session_id: 's', generation: 'generation-s', request_id: pending.id, response: { decision: 'accept' } } }]]);
 expect(invoke.mock.calls.some(([command]) => String(command).includes('capability_confirmation'))).toBe(false);
});

it('keeps a first Claude choice unbound until an explicit Provider selection and saves that engine', async () => {
 invoke.mockImplementation(async (command: string, args: { engine?: string; input?: { configuration: unknown } }) => {
  if (command === 'read_development_cli_provider') return null;
  if (command === 'read_development_cli_provider_catalog') return { project_id: 'p', engine: args.engine, providers: [{ id: 'messages-provider', name: 'Messages Provider', enabled: true, config_revision: 7, availability: { available: true, reason: null }, models: [{ id: 'claude-model', name: 'Claude model', model_type: 'chat' }] }] };
  if (command === 'save_development_cli_provider') return { revision: 1, permission_epoch: 0, configuration: args.input?.configuration, created_at: 1 };
  return null;
 });
 const view = render(<MemoryRouter><ProjectCapabilities projectId="p" /></MemoryRouter>);
 fireEvent.click(await screen.findByRole('combobox', { name: 'agent.engine' }));
 fireEvent.click(await screen.findByRole('option', { name: 'Claude' }));
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('read_development_cli_provider_catalog', { projectId: 'p', engine: 'claude' }));
 expect(screen.getByRole('combobox', { name: 'agent.engine' }).textContent).toContain('Claude');
 expect(screen.getByRole('button', { name: 'capabilities.save' }).hasAttribute('disabled')).toBe(true);
 view.unmount();
 render(<MemoryRouter><ProjectCapabilities projectId="p" /></MemoryRouter>);
 const source = await screen.findByRole('combobox', { name: 'capabilities.source' });
 expect(screen.getByRole('combobox', { name: 'agent.engine' }).textContent).toContain('Claude');
 fireEvent.change(source, { target: { value: 'messages-provider' } });
 fireEvent.click(screen.getByRole('button', { name: 'capabilities.save' }));
 await waitFor(() => expect(invoke).toHaveBeenCalledWith('save_development_cli_provider', { input: { project_id: 'p', expected_revision: 0, configuration: { engine: 'claude', provider: { provider_id: 'messages-provider', model_id: 'claude-model', provider_revision: 7 } } } }));
});

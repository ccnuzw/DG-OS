import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { ProjectReview } from '../src/components/development/ProjectReview';
import { Request } from '../src/pages/workbench/AgentConversation';
import { useDevelopment } from '../src/stores/developmentStore';
import { chooseContext } from '../src/stores/developmentSelection';
import { ProjectCapabilities } from '../src/components/development/ProjectCapabilities';
import { HostCapabilityConfirmations } from '../src/components/development/HostCapabilityConfirmations';
import { useAgentRunner } from '../src/stores/agentRunnerStore';
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
  if (cmd === 'read_development_capability_catalog') return ++reads === 1 ? catalog.promise : Promise.resolve({ providers: [], skills: [], mcp: [] });
  return Promise.resolve(null);
 });
 render(<ProjectCapabilities projectId="p" />);
 await act(async () => { useDevelopment.getState().reset(); });
 await act(async () => { catalog.resolve({ providers: [{ id: 'private-provider', name: 'PRIVATE_OLD_PROVIDER', provider_type: 'openai', config_revision: 1, enabled: true, models: [{ id: 'm', name: 'm', model_type: 'chat' }] }], skills: [], mcp: [] }); });
 expect(document.body.textContent).not.toContain('PRIVATE_OLD_PROVIDER');
});

it('keeps native approvals separate and never replays a consumed host confirmation', async () => {
 vi.useFakeTimers();
 const item = { id: 'host-confirmation', project_id: 'p', task_id: 't', session_id: 's', run_id: 'r', server_id: 'server', tool_name: 'write', schema_hash: 'schema-v1', arguments: { value: 'review-me' } };
 invoke.mockImplementation((cmd: string) => cmd === 'list_development_capability_confirmations' ? Promise.resolve([item]) : Promise.resolve(null));
 useAgentRunner.setState({ selected: 's' });
 const native = vi.fn(async () => true);
 await act(async () => { render(<><Request engine="codex" pending={{ id: 'native', kind: 'item/commandExecution/requestApproval', title: 'native', data: {} }} submit={native} /><HostCapabilityConfirmations sessionId="s" /></>); });
 const host = screen.getByRole('region', { name: 'capabilities.hostApproval' });
 const allow = Array.from(host.querySelectorAll('button')).find(button => button.textContent === 'agent.allow')!;
 await act(async () => { fireEvent.click(allow); });
 await act(async () => { await vi.advanceTimersByTimeAsync(750); });
 expect(screen.queryByRole('region', { name: 'capabilities.hostApproval' })).toBeNull();
 expect(screen.getByRole('region', { name: 'agent.approval' })).toBeTruthy();
 expect(native).not.toHaveBeenCalled();
 expect(invoke.mock.calls.filter(([cmd]) => cmd === 'respond_development_capability_confirmation')).toHaveLength(1);
});

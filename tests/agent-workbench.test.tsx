import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import AgentWorkbench from '../src/pages/workbench/AgentWorkbench';
import { useAgentRunner } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));
afterEach(() => { cleanup(); useAgentRunner.getState().reset(); useDevelopment.getState().reset(); invoke.mockReset(); });

const workspace = {
 projects: [{ id: 'p1', owner: 'u', name: 'demo', path: '/tmp/project', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: 1 }],
 tasks: [{ id: 't1', owner: 'u', project_id: 'p1', goal: 'Ship', requirements: 'Tests pass', status: 'todo', created_at: 1 }],
 sessions: [], selection: { project_id: 'p1', task_id: 't1', session_id: null }, operations: [],
};
const inspection = { path_state: 'available', kind: 'directory', repository_root: null, branch: null, changes: [], truncated: false, error: null };
const session = { id: 'one', engine: 'codex', native_id: 'native', workspace: '/tmp/project', status: 'idle', events: [], pending: [], error: null };

// `override` returns a value for the commands a test cares about; undefined falls back to the defaults below.
function install(override: (command: string) => unknown = () => undefined) {
 invoke.mockImplementation(async (command: string) => {
  const result = override(command);
  if (result !== undefined) return result;
  if (command === 'read_development_workspace') return workspace;
  if (command === 'inspect_development_project') return inspection;
  if (command === 'open_agent_session' || command === 'get_agent_session') return session;
  if (command === 'read_development_capabilities' || command === 'get_development_session_capabilities') return null;
  if (command === 'list_development_capability_confirmations') return [];
  if (command === 'read_development_capability_catalog') return { providers: [], skills: [], mcp: [] };
  return undefined;
 });
}
// Trust and start are explicit clicks: the project directory must be trusted before a runtime can be created.
async function start(state = 'idle') {
 await screen.findByTestId('dev-task-summary');
 fireEvent.click(screen.getByTestId('dev-trust'));
 fireEvent.click(screen.getByTestId('dev-session-new-codex'));
 await screen.findByText(`agent.state.${state}`);
}
describe('agent sessions', () => {
 it('requires workspace trust before creating a runtime', async () => {
  install(); render(<AgentWorkbench />);
  const button = await screen.findByTestId('dev-session-new-codex') as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  expect(invoke.mock.calls.some(([name]) => name === 'open_agent_session')).toBe(false);
 });
 it('keeps the host session alive when its page unmounts', async () => {
  install(); const view = render(<AgentWorkbench />); await start(); view.unmount();
  expect(invoke.mock.calls.some(([name]) => name === 'close_agent_session' || name === 'interrupt_agent_session')).toBe(false);
 });
 it('retains unsent text on submission failure and does not retry', async () => {
  install(); render(<AgentWorkbench />); await start();
  install(command => { if (command === 'send_agent_message') throw Error('private-token'); return undefined; });
  fireEvent.change(screen.getByLabelText('agent.prompt'), { target: { value: 'change file' } });
  fireEvent.click(screen.getByRole('button', { name: 'agent.send' }));
  await screen.findAllByRole('alert');
  expect((screen.getByLabelText('agent.prompt') as HTMLTextAreaElement).value).toBe('change file');
  expect(document.body.textContent).not.toContain('private-token');
  expect(invoke.mock.calls.filter(([name]) => name === 'send_agent_message')).toHaveLength(1);
 });
 it('returns a Claude denial without translating it to a Codex decision', async () => {
  const claude = { ...session, engine: 'claude', status: 'waiting', pending: [{ id: 'approval', kind: 'claude/toolPermission', title: 'Bash', data: { input: { command: 'echo hello' } } }] };
  install(command => (command === 'open_agent_session' || command === 'get_agent_session' ? claude : undefined));
  render(<AgentWorkbench />); await start('waiting');
  fireEvent.click(await screen.findByRole('button', { name: 'agent.deny' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('respond_agent_request', { input: { session_id: 'one', request_id: 'approval', response: { behavior: 'deny' } } }));
 });
});

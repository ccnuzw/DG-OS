import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { clearToasts } from '@dgos/ui';
import { getToastSnapshot } from '../src/ui/feedback/toastStore';
import Workbench from '../src/pages/Workbench';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);
const skill = { id: 'summary@1.0.0', skill_id: 'summary', name: 'Summary', version: '1.0.0', enabled: true, call_type: 'content' };
const record = { task: { id: 'task-1', status: 'succeeded', provider_id: 'p1', model_id: 'chat1', created_at: 1, total_tokens: 15 },
  skills: [{ skill_id: 'summary', version_id: 'summary@1.0.0', content_hash: 'a'.repeat(64) }] };

beforeEach(() => {
  mocked.mockImplementation(async command => {
    if (command === 'list_providers') return { success: true, data: [{ id: 'p1', name: 'My AI', enabled: true, credential_status: 'stored' }] };
    if (command === 'list_models') return { success: true, data: [{ id: 'm1', model_id: 'chat1', provider_id: 'p1', model_name: 'Chat One', model_type: 'chat' }] };
    if (command === 'list_installed_skills') return [skill, { ...skill, id: 'disabled', skill_id: 'disabled', name: 'Disabled skill', enabled: false }];
    if (command === 'list_workbench_tasks') return [];
    if (command === 'list_mcp_servers') return [];
    if (command === 'run_workbench_task') return { record, content: 'The release is Friday.' };
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(() => { cleanup(); clearToasts(); vi.restoreAllMocks(); });
const show = () => {
  const view = render(<MemoryRouter><Workbench /></MemoryRouter>);
  fireEvent.mouseDown(screen.getByRole('tab', { name: '单任务' }), { button: 0 });
  return view;
};
async function open(name: string) {
  const trigger = await screen.findByRole('combobox', { name });
  await waitFor(() => expect((trigger as HTMLButtonElement).disabled).toBe(false));
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  return within(await screen.findByRole('listbox'));
}
async function choose(name: string, option: string | RegExp) {
  fireEvent.click((await open(name)).getByRole('option', { name: option }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
}
async function configure() {
  await waitFor(() => expect(screen.queryByText('Loading local configuration...')).toBeNull());
  await choose('Provider', 'My AI');
  await choose('Model', 'Chat One');
  fireEvent.change(screen.getByLabelText('Task input'), { target: { value: 'Summarize the release notes.' } });
}

it('AC20 sends only the explicitly selected skill and shows the exact bound version', async () => {
  show();
  await configure();
  const skills = await open('Skill for this task');
  expect(skills.queryByRole('option', { name: /Disabled skill/ })).toBeNull();
  fireEvent.click(skills.getByRole('option', { name: 'Summary — v1.0.0' }));
  fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
  await screen.findByText('The release is Friday.');
  expect(mocked).toHaveBeenCalledWith('run_workbench_task', { input: expect.objectContaining({
    provider_id: 'p1', model_id: 'chat1', prompt: 'Summarize the release notes.', skill_id: 'summary', allow_without_skill: false,
  }) });
  expect(mocked.mock.calls.find(([name]) => name === 'run_workbench_task')?.[1]).toEqual({
    input: expect.objectContaining({ client_task_id: expect.stringMatching(/^[0-9a-f-]{36}$/) }),
  });
  expect(screen.getByText(/Bound version: summary@1.0.0/)).toBeTruthy();
  expect(mocked.mock.calls.some(([name]) => name === 'call_provider')).toBe(false);
});

it('AC22 requires explicit confirmation to run without a skill', async () => {
  show();
  await configure();
  const run = screen.getByRole('button', { name: 'Run without a skill' });
  expect(run.hasAttribute('disabled')).toBe(true);
  fireEvent.click(run);
  expect(mocked.mock.calls.filter(([name]) => name === 'run_workbench_task')).toHaveLength(0);
  fireEvent.click(screen.getByRole('checkbox', { name: 'I explicitly want to run without a skill' }));
  fireEvent.click(run);
  await waitFor(() => expect(mocked).toHaveBeenCalledWith('run_workbench_task', expect.objectContaining({ input: expect.objectContaining({ skill_id: null, allow_without_skill: true }) })));
});

it('AC22 reports a rejected skill without sending a fallback or showing stale output', async () => {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'run_workbench_task' ? Promise.reject('Selected skill unavailable') : handler(command, args));
  show();
  await configure();
  await choose('Skill for this task', /^Summary/);
  fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
  await screen.findByRole('alert');
  expect(screen.getByText(/No fallback request was sent/)).toBeTruthy();
  expect(screen.queryByText('The release is Friday.')).toBeNull();
  expect(mocked.mock.calls.filter(([name]) => name === 'run_workbench_task')).toHaveLength(1);
});

it('AC21 regression prevents duplicate sends while a request is running', async () => {
  let resolve!: (value: unknown) => void;
  const pending = new Promise(r => { resolve = r; });
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'run_workbench_task' ? pending : handler(command, args));
  show();
  await configure();
  await choose('Skill for this task', /^Summary/);
  const button = screen.getByRole('button', { name: 'Run with selected skill' });
  fireEvent.click(button);
  fireEvent.click(button);
  expect(screen.getByRole('button', { name: 'Running task...' }).hasAttribute('disabled')).toBe(true);
  expect(mocked.mock.calls.filter(([name]) => name === 'run_workbench_task')).toHaveLength(1);
  resolve({ record, content: 'Finished once.' });
  await screen.findByText('Finished once.');
});

it('text task cancellation uses the host text-task command and does not replay', async () => {
  let resolve!: (value: unknown) => void;
  const pending = new Promise(r => { resolve = r; });
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'run_workbench_task') return pending;
    if (command === 'cancel_workbench_task') {
      return Promise.resolve(true);
    }
    return handler(command, args);
  });
  show();
  await configure();
  await choose('Skill for this task', /^Summary/);
  fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
  await screen.findByRole('button', { name: 'Cancel task' });
  fireEvent.click(screen.getByRole('button', { name: 'Cancel task' }));
  await waitFor(() => expect(mocked).toHaveBeenCalledWith('cancel_workbench_task', {
    taskId: expect.stringMatching(/^[0-9a-f-]{36}$/),
  }));
  expect(mocked.mock.calls.some(([name]) => name === 'cancel_workbench_tool_task')).toBe(false);
  resolve({ record: { ...record, task: { ...record.task, status: 'interrupted', error_code: 'task_cancelled' } }, content: null });
});

it('AC22 rejects oversized UTF-8 input before invoking the backend', async () => {
  show();
  await configure();
  await choose('Skill for this task', /^Summary/);
  fireEvent.change(screen.getByLabelText('Task input'), { target: { value: '中'.repeat(6000) } });
  expect(screen.getByRole('button', { name: 'Run with selected skill' }).hasAttribute('disabled')).toBe(true);
  expect(mocked.mock.calls.some(([name]) => name === 'run_workbench_task')).toBe(false);
});

it('AC35 shows failed task audit and never presents it as model success', async () => {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'run_workbench_task'
    ? Promise.resolve({ record: { ...record, task: { ...record.task, status: 'failed', error_code: 'provider_1001002' } }, content: null }) : handler(command, args));
  show();
  await configure();
  await choose('Skill for this task', /^Summary/);
  fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
  await screen.findByRole('alert');
  expect(screen.getByText(/Task failed: provider_1001002/)).toBeTruthy();
  expect(screen.queryByText('The release is Friday.')).toBeNull();
});

const selectedTool = { server_id: 'server-1', tool_name: 'echo', description: 'Echo fixture data', schema_hash: 'b'.repeat(64), input_schema: { type: 'object' }, discovered_at: 1 };
const proposal = { id: 'proposal-1', server_id: 'server-1', server_name: 'Local tools', tool_name: 'echo', schema_hash: selectedTool.schema_hash,
  arguments: { text: 'Review these exact arguments' }, expires_at: Math.floor(Date.now() / 1000) + 120 };
const toolRecord = { ...record, task: { ...record.task, status: 'running' }, tools: [selectedTool] };
function mockTools() {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation(async (command, args) => {
    if (command === 'list_mcp_servers') return [{ id: 'server-1', name: 'Local tools', enabled: true, status: 'ready' }, { id: 'offline', enabled: true, status: 'error' }];
    if (command === 'list_mcp_tools') return [selectedTool, { ...selectedTool, tool_name: 'unselected' }];
    if (command === 'start_workbench_tool_task') return { record: toolRecord, phase: 'ready' };
    if (command === 'advance_workbench_tool_task') {
      const action = args?.action as { type: string };
      if (action.type === 'run_model') return { record: toolRecord, phase: 'awaiting_confirmation', proposal };
      return { record: { ...toolRecord, task: { ...toolRecord.task, status: action.type === 'approve' ? 'succeeded' : 'failed', error_code: action.type === 'deny' ? 'tool_denied' : undefined } }, phase: 'finished', content: action.type === 'approve' ? 'Tool-assisted answer' : undefined };
    }
    if (command === 'cancel_workbench_tool_task') return { record: { ...toolRecord, task: { ...toolRecord.task, status: 'failed', error_code: 'task_cancelled' } }, phase: 'finished' };
    return handler(command, args);
  });
}
async function configureTools() {
  await configure();
  await choose('Skill for this task', /^Summary/);
  fireEvent.click(await screen.findByRole('checkbox', { name: /Local tools \/ echo/ }));
  fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
  await screen.findByRole('heading', { name: 'Review proposed tool call' });
}

it('MCP selection is explicit and approval sends only the host proposal ID', async () => {
  mockTools(); show(); await configureTools();
  expect(screen.getByLabelText('Exact tool arguments').textContent).toContain('Review these exact arguments');
  // The page moves focus to the heading in an effect after it renders, so wait for it rather than reading it at once.
  await waitFor(() => expect(screen.getByRole('heading', { name: 'Review proposed tool call' })).toBe(document.activeElement));
  expect(mocked).toHaveBeenCalledWith('start_workbench_tool_task', { input: {
    task: expect.objectContaining({ provider_id: 'p1', model_id: 'chat1', prompt: 'Summarize the release notes.', skill_id: 'summary', allow_without_skill: false }),
    tools: [{ server_id: 'server-1', tool_name: 'echo', schema_hash: selectedTool.schema_hash }],
  } });
  expect(mocked.mock.calls.find(([name]) => name === 'start_workbench_tool_task')?.[1]).toEqual({
    input: expect.objectContaining({ task: expect.objectContaining({ client_task_id: expect.stringMatching(/^[0-9a-f-]{36}$/) }) }),
  });
  expect(mocked.mock.calls.filter(([name, args]) => name === 'advance_workbench_tool_task' && (args?.action as {type: string}).type === 'approve')).toHaveLength(0);
  const approve = screen.getByRole('button', { name: 'Approve this call once' });
  fireEvent.click(approve); fireEvent.click(approve);
  await screen.findByText('Tool-assisted answer');
  expect(mocked).toHaveBeenCalledWith('advance_workbench_tool_task', { taskId: 'task-1', action: { type: 'approve', proposal_id: 'proposal-1' } });
  expect(mocked.mock.calls.filter(([name, args]) => name === 'advance_workbench_tool_task' && (args?.action as {type: string}).type === 'approve')).toHaveLength(1);
  expect(mocked.mock.calls.some(([name]) => ['confirm_mcp_tool_call', 'call_mcp_tool', 'call_provider', 'run_workbench_task'].includes(name))).toBe(false);
});

it('MCP reject stops the task without approving or presenting success', async () => {
  mockTools(); show(); await configureTools();
  fireEvent.click(screen.getByRole('button', { name: 'Reject and stop' }));
  await screen.findByRole('alert');
  expect(screen.getByText(/Task failed: tool_denied/)).toBeTruthy();
  expect(screen.queryByText('Tool-assisted answer')).toBeNull();
  expect(mocked.mock.calls.filter(([name, args]) => name === 'advance_workbench_tool_task' && (args?.action as {type: string}).type === 'approve')).toHaveLength(0);
});

it('MCP unknown external result is explicit and never presented as success', async () => {
  mockTools(); show(); await configureTools();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'advance_workbench_tool_task'
    && (args?.action as { type: string }).type === 'approve'
    ? Promise.resolve({ record: { ...toolRecord, task: { ...toolRecord.task, status: 'failed', error_code: 'mcp_unknown_result' } }, phase: 'finished' })
    : handler(command, args));
  fireEvent.click(screen.getByRole('button', { name: 'Approve this call once' }));
  await screen.findByRole('alert');
  expect(screen.getByText(/external tool result is unknown; do not retry automatically/i)).toBeTruthy();
  expect(screen.queryByText('Tool-assisted answer')).toBeNull();
});

it('MCP task cancels on leaving the page without replaying approval', async () => {
  mockTools(); const view = show(); await configureTools();
  view.unmount();
  expect(mocked).toHaveBeenCalledWith('cancel_workbench_tool_task', { taskId: 'task-1' });
});

it('MCP cancellation clears the proposal and explains that external effects are not undone', async () => {
  mockTools(); show(); await configureTools();
  fireEvent.click(screen.getByRole('button', { name: 'Cancel tool task' }));
  await screen.findByRole('alert');
  expect(screen.getByText(/Cancellation does not undo external effects/)).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Approve this call once' })).toBeNull();
});

it('choosing "No skill selected" again clears the skill and requires the explicit no-skill confirmation', async () => {
  show();
  await configure();
  await choose('Skill for this task', /^Summary/);
  expect(screen.queryByRole('checkbox', { name: 'I explicitly want to run without a skill' })).toBeNull();
  await choose('Skill for this task', 'No skill selected');
  expect(screen.getByRole('checkbox', { name: 'I explicitly want to run without a skill' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Run without a skill' }).hasAttribute('disabled')).toBe(true);
});

it('a provider audit warning stays visible as an inline alert and is not turned into a toast', async () => {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'run_workbench_task'
    ? Promise.resolve({ record, content: 'Done.', audit_warning: true }) : handler(command, args));
  show();
  await configure();
  await choose('Skill for this task', /^Summary/);
  fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
  expect((await screen.findByRole('alert')).textContent).toMatch(/usage audit persistence failed/);
  expect(getToastSnapshot().visible).toHaveLength(0);
});

describe('failure reporting through the unified handler', () => {
  beforeEach(() => { vi.spyOn(console, 'error').mockImplementation(() => undefined); });
  const coded = (code: number, text: string) => Object.assign(new Error(text), { code });

  it('a rejected configuration load keeps its inline text and shows the registered toast for the envelope code', async () => {
    const handler = mocked.getMockImplementation()!;
    mocked.mockImplementation((command, args) => command === 'list_providers'
      ? Promise.resolve({ success: false, error: { code: 1001002, message: 'raw host detail' } }) : handler(command, args));
    show();
    expect((await screen.findByRole('alert')).textContent).toContain('Unable to load workbench: raw host detail');
    await waitFor(() => expect(getToastSnapshot().visible.map(item => item.message)).toEqual(['API Key 无效']));
    expect(JSON.stringify(getToastSnapshot())).not.toContain('raw host detail');
  });

  it('a failed run reports its coded error once and keeps the inline failure text', async () => {
    const handler = mocked.getMockImplementation()!;
    mocked.mockImplementation((command, args) => command === 'run_workbench_task'
      ? Promise.reject(coded(1001006, 'upstream timeout')) : handler(command, args));
    show();
    await configure();
    await choose('Skill for this task', /^Summary/);
    fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
    expect((await screen.findByRole('alert')).textContent).toMatch(/Task not completed: upstream timeout.*No fallback request was sent/);
    expect(getToastSnapshot().visible.map(item => item.message)).toEqual(['请求超时']);
  });

  it('a cancellation the host cannot confirm reports its error', async () => {
    let resolve!: (value: unknown) => void;
    const pending = new Promise(r => { resolve = r; });
    const handler = mocked.getMockImplementation()!;
    mocked.mockImplementation((command, args) => {
      if (command === 'run_workbench_task') return pending;
      if (command === 'cancel_workbench_task') return Promise.reject(coded(1001006, 'cancel timeout'));
      return handler(command, args);
    });
    show();
    await configure();
    await choose('Skill for this task', /^Summary/);
    fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel task' }));
    expect((await screen.findByRole('alert')).textContent).toMatch(/Cancellation not confirmed: cancel timeout/);
    expect(getToastSnapshot().visible.map(item => item.message)).toEqual(['请求超时']);
    resolve({ record, content: null });
  });
});

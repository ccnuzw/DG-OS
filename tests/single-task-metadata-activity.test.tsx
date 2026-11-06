import { StrictMode } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { clearToasts } from '@dgos/ui';
import Workbench from '../src/pages/workbench/SingleTaskWorkbench';
import { setLocaleForTesting } from '../src/i18n/runtime';
import { useTaskRunner } from '../src/stores/taskRunnerStore';
import { getToastSnapshot } from '../src/ui/feedback/toastStore';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);
const provider = { id: 'p1', name: 'My AI', enabled: true, credential_status: 'stored' };
const otherProvider = { ...provider, id: 'p2', name: 'Other AI' };
const thirdProvider = { ...provider, id: 'p3', name: 'Third AI' };
const model = { id: 'm1', model_id: 'chat1', provider_id: 'p1', model_name: 'Chat One', model_type: 'chat' };
const skill = { id: 'summary@1.0.0', skill_id: 'summary', name: 'Summary', version: '1.0.0', enabled: true, call_type: 'content' };
const record = { task: { id: 'task-1', status: 'succeeded', provider_id: 'p1', model_id: 'saved-model', created_at: 1 }, skills: [] };
const tool = { server_id: 'server-1', tool_name: 'echo', schema_hash: 'a'.repeat(64), description: 'Saved tool' };
const server = { id: 'server-1', name: 'Local tools', enabled: true, status: 'ready' };
const metadataCommands = ['list_providers', 'list_installed_skills', 'list_workbench_tasks', 'list_mcp_servers'];

async function flush() { await act(async () => {}); }
function show(active = true, strict = false) {
  const controller = createWorkspaceController();
  const tree = (value: boolean) => {
    const content = <MemoryRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
      <SurfaceScope id="single-task" active={value} controller={controller}><Workbench /></SurfaceScope>
    </MemoryRouter>;
    return strict ? <StrictMode>{content}</StrictMode> : content;
  };
  const view = render(tree(active));
  return { ...view, setActive: async (value: boolean) => { view.rerender(tree(value)); await flush(); } };
}
async function choose(name: string, option: string) {
  const trigger = screen.getByRole('combobox', { name });
  await waitFor(() => expect((trigger as HTMLButtonElement).disabled).toBe(false));
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  fireEvent.click(within(await screen.findByRole('listbox')).getByRole('option', { name: option }));
  await flush();
}

beforeEach(() => {
  setLocaleForTesting('en-US');
  useTaskRunner.getState().reset();
  mocked.mockImplementation(async (command, args) => {
    if (command === 'list_providers') return { success: true, data: [provider, otherProvider, thirdProvider] };
    if (command === 'list_models') return { success: true, data: [{ ...model, provider_id: args?.providerId }] };
    if (command === 'list_installed_skills') return [skill];
    if (command === 'list_workbench_tasks') return [record];
    if (command === 'list_mcp_servers') return [server];
    if (command === 'list_mcp_tools') return [tool];
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(() => {
  cleanup();
  useTaskRunner.getState().reset();
  clearToasts();
  mocked.mockReset();
  vi.restoreAllMocks();
});

it('does no presentation reads on inactive mount and starts one snapshot on activation', async () => {
  const view = show(false); await flush();
  expect(mocked).not.toHaveBeenCalled();
  await view.setActive(true);
  for (const command of metadataCommands) expect(mocked.mock.calls.filter(([name]) => name === command)).toHaveLength(1);
  expect(screen.getByText('task-1')).toBeTruthy();
  expect(screen.getByRole('checkbox', { name: /Local tools \/ echo/ })).toBeTruthy();
});

it('starts one initial snapshot under StrictMode without invalidating it during effect replay', async () => {
  show(true, true); await flush();
  for (const command of metadataCommands) expect(mocked.mock.calls.filter(([name]) => name === command)).toHaveLength(1);
  expect(screen.getByRole('checkbox', { name: 'Summary — v1.0.0' })).toBeTruthy();
});

it('joins slow metadata across repeated restores and refreshes and accepts the response while inactive', async () => {
  const tools = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'list_mcp_tools' ? tools.promise : handler(command, args));
  const view = show(); await flush();
  for (let index = 0; index < 3; index++) {
    await view.setActive(false); await view.setActive(true);
    fireEvent.click(screen.getByRole('button', { name: 'Refresh configuration' })); await flush();
  }
  for (const command of metadataCommands) expect(mocked.mock.calls.filter(([name]) => name === command)).toHaveLength(1);
  expect(mocked.mock.calls.filter(([name]) => name === 'list_mcp_tools')).toHaveLength(1);
  await view.setActive(false);
  await act(async () => { tools.resolve([tool]); });
  expect(screen.getByRole('checkbox', { name: /Local tools \/ echo/ })).toBeTruthy();
  await view.setActive(true);
  for (const command of metadataCommands) expect(mocked.mock.calls.filter(([name]) => name === command)).toHaveLength(2);
});

it('defers tool discovery that would start after becoming inactive until restoration', async () => {
  const servers = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  let pending = true;
  mocked.mockImplementation((command, args) => command === 'list_mcp_servers' && pending ? servers.promise : handler(command, args));
  const view = show(); await flush(); await view.setActive(false);
  await act(async () => { servers.resolve([server]); });
  expect(mocked.mock.calls.filter(([name]) => name === 'list_mcp_tools')).toHaveLength(0);
  pending = false;
  await view.setActive(true);
  expect(mocked.mock.calls.filter(([name]) => name === 'list_mcp_tools')).toHaveLength(1);
  expect(screen.getByRole('checkbox', { name: /Local tools \/ echo/ })).toBeTruthy();
});

it('joins same-provider model reads and keeps their valid response across inactivity', async () => {
  const models = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'list_models' ? models.promise : handler(command, args));
  const view = show(); await flush(); await choose('Provider', 'My AI');
  for (let index = 0; index < 3; index++) {
    await view.setActive(false); await view.setActive(true);
    fireEvent.click(screen.getByRole('button', { name: 'Refresh configuration' })); await flush();
  }
  expect(mocked.mock.calls.filter(([name]) => name === 'list_models')).toHaveLength(1);
  await view.setActive(false);
  await act(async () => { models.resolve({ success: true, data: [model] }); });
  await choose('Model', 'Chat One');
  expect(screen.getByRole('combobox', { name: 'Model' }).textContent).toContain('Chat One');
  expect(mocked.mock.calls.filter(([name]) => name === 'list_models')).toHaveLength(1);
});

it('performs one trailing latest snapshot when slow metadata and models span repeated restores', async () => {
  const history = Promise.withResolvers<unknown>();
  const models = Promise.withResolvers<unknown>();
  let latest = false;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'list_workbench_tasks') return latest ? Promise.resolve([{ ...record, task: { ...record.task, id: 'latest-task' } }]) : history.promise;
    if (command === 'list_models') return latest ? Promise.resolve({ success: true, data: [{ ...model, model_name: 'Latest model' }] }) : models.promise;
    return handler(command, args);
  });
  const view = show(); await flush(); await choose('Provider', 'My AI');
  for (let index = 0; index < 4; index++) {
    await view.setActive(false); await view.setActive(true);
    fireEvent.click(screen.getByRole('button', { name: 'Refresh configuration' })); await flush();
  }
  for (const command of [...metadataCommands, 'list_models']) expect(mocked.mock.calls.filter(([name]) => name === command)).toHaveLength(1);
  latest = true;
  await act(async () => {
    history.resolve([record]);
    models.resolve({ success: true, data: [model] });
  });
  expect(screen.getByText('latest-task')).toBeTruthy();
  expect(screen.queryByText('task-1')).toBeNull();
  await choose('Model', 'Latest model');
  for (const command of [...metadataCommands, 'list_models']) expect(mocked.mock.calls.filter(([name]) => name === command)).toHaveLength(2);
});

it('coalesces slow provider switches to the latest target and excludes cross-provider models', async () => {
  const first = Promise.withResolvers<unknown>();
  const latest = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'list_models') return args?.providerId === 'p1' ? first.promise : latest.promise;
    return handler(command, args);
  });
  show(); await flush(); await choose('Provider', 'My AI');
  await choose('Provider', 'Other AI'); await choose('Provider', 'Third AI');
  expect(mocked.mock.calls.filter(([name]) => name === 'list_models')).toHaveLength(1);
  await act(async () => { first.resolve({ success: true, data: [model] }); });
  expect(mocked.mock.calls.filter(([name]) => name === 'list_models').map(([, args]) => args?.providerId)).toEqual(['p1', 'p3']);
  await act(async () => { latest.resolve({ success: true, data: [model, { ...model, id: 'm3', provider_id: 'p3', model_id: 'third', model_name: 'Third model' }] }); });
  const trigger = screen.getByRole('combobox', { name: 'Model' });
  fireEvent.keyDown(trigger, { key: 'Enter' });
  expect(within(await screen.findByRole('listbox')).queryByRole('option', { name: 'Chat One' })).toBeNull();
  fireEvent.click(screen.getByRole('option', { name: 'Third model' }));
  expect(trigger.textContent).toContain('Third model');
});

it('keeps the previous configuration, models, tools and history after failed refresh and retries on restore', async () => {
  let failing = false;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => failing ? Promise.reject(new Error('Offline')) : handler(command, args));
  const view = show(); await flush(); await choose('Provider', 'My AI'); await choose('Model', 'Chat One');
  fireEvent.change(screen.getByLabelText('Task input'), { target: { value: 'Keep this draft' } });
  fireEvent.click(screen.getByRole('checkbox', { name: /Local tools \/ echo/ }));
  failing = true;
  fireEvent.click(screen.getByRole('button', { name: 'Refresh configuration' })); await flush();
  expect(screen.getByRole('combobox', { name: 'Provider' }).textContent).toContain('My AI');
  expect(screen.getByRole('combobox', { name: 'Model' }).textContent).toContain('Chat One');
  expect(screen.getByRole('checkbox', { name: /Local tools \/ echo/ }).getAttribute('aria-checked')).toBe('true');
  expect(screen.getByRole('checkbox', { name: 'Summary — v1.0.0' })).toBeTruthy();
  expect(screen.getByText('task-1')).toBeTruthy();
  expect((screen.getByLabelText('Task input') as HTMLTextAreaElement).value).toBe('Keep this draft');
  failing = false;
  await view.setActive(false); await view.setActive(true);
  expect(screen.queryAllByRole('alert')).toHaveLength(0);
  for (const command of metadataCommands) expect(mocked.mock.calls.filter(([name]) => name === command)).toHaveLength(3);
});

it('clears account-owned metadata and rejects old-account responses before reading the replacement account', async () => {
  const oldModels = Promise.withResolvers<unknown>();
  const oldProviders = Promise.withResolvers<unknown>();
  const oldTools = Promise.withResolvers<unknown>();
  const oldHistory = Promise.withResolvers<unknown>();
  let oldAccountRead = false;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'list_models') return oldModels.promise;
    if (oldAccountRead && command === 'list_providers') return oldProviders.promise;
    if (oldAccountRead && command === 'list_mcp_servers') return oldTools.promise;
    if (oldAccountRead && command === 'list_workbench_tasks') return oldHistory.promise;
    return handler(command, args);
  });
  const view = show(); await flush(); await choose('Provider', 'My AI');
  fireEvent.change(screen.getByLabelText('Task input'), { target: { value: 'Private draft' } });
  oldAccountRead = true;
  fireEvent.click(screen.getByRole('button', { name: 'Refresh configuration' })); await flush();
  await view.setActive(false);
  await act(async () => { useTaskRunner.getState().reset(); });
  expect(screen.queryByText('task-1')).toBeNull();
  expect((screen.getByLabelText('Task input') as HTMLTextAreaElement).value).toBe('');
  expect(screen.queryByRole('checkbox', { name: /Local tools \/ echo/ })).toBeNull();
  await view.setActive(true); await view.setActive(false); await view.setActive(true);
  for (const command of metadataCommands) expect(mocked.mock.calls.filter(([name]) => name === command)).toHaveLength(2);
  oldAccountRead = false;
  await act(async () => {
    oldModels.resolve({ success: true, data: [model] });
    oldProviders.resolve({ success: true, data: [{ ...provider, name: 'Private old provider' }] });
    oldTools.reject(new Error('Private old failure'));
    oldHistory.resolve([{ ...record, task: { ...record.task, id: 'private-old-task' } }]);
  });
  expect(screen.queryByText('private-old-task')).toBeNull();
  expect(screen.queryAllByRole('alert')).toHaveLength(0);
  expect(getToastSnapshot().visible).toHaveLength(0);
  for (const command of metadataCommands) expect(mocked.mock.calls.filter(([name]) => name === command)).toHaveLength(3);
  expect(screen.getByRole('combobox', { name: 'Provider' }).textContent).not.toContain('Private old provider');
  await choose('Provider', 'My AI');
  await choose('Model', 'Chat One');
});

it('does not apply or report metadata after unmount', async () => {
  const pending = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => metadataCommands.includes(command) ? pending.promise : handler(command, args));
  const view = show(); await flush(); view.unmount();
  await act(async () => { pending.reject(new Error('Late failure')); });
  expect(getToastSnapshot().visible).toHaveLength(0);
});

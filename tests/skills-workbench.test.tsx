import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import Workbench from '../src/pages/Workbench';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);
const skill = { id: 'summary@1.0.0', skill_id: 'summary', name: 'Summary', version: '1.0.0', enabled: true, call_type: 'content' };
const record = { task: { id: 'task-1', status: 'succeeded', provider_id: 'p1', model_id: 'chat1', created_at: 1, total_tokens: 15 },
  skills: [{ skill_id: 'summary', version_id: 'summary@1.0.0', content_hash: 'a'.repeat(64) }] };

beforeEach(() => {
  mocked.mockImplementation(async command => {
    if (command === 'list_providers') return { success: true, data: [{ id: 'p1', name: 'My AI', enabled: true }] };
    if (command === 'list_models') return { success: true, data: [{ id: 'm1', model_id: 'chat1', provider_id: 'p1', model_name: 'Chat One', model_type: 'chat' }] };
    if (command === 'list_installed_skills') return [skill, { ...skill, id: 'disabled', skill_id: 'disabled', name: 'Disabled skill', enabled: false }];
    if (command === 'list_workbench_tasks') return [];
    if (command === 'run_workbench_task') return { record, content: 'The release is Friday.' };
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });
const show = () => render(<MemoryRouter><Workbench /></MemoryRouter>);
async function configure() {
  await screen.findByRole('option', { name: 'My AI' });
  fireEvent.change(screen.getByLabelText('Provider'), { target: { value: 'p1' } });
  await screen.findByRole('option', { name: 'Chat One' });
  fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'chat1' } });
  fireEvent.change(screen.getByLabelText('Task input'), { target: { value: 'Summarize the release notes.' } });
}

it('AC20 sends only the explicitly selected skill and shows the exact bound version', async () => {
  show();
  await configure();
  expect(screen.queryByRole('option', { name: /Disabled skill/ })).toBeNull();
  fireEvent.change(screen.getByLabelText('Skill for this task'), { target: { value: 'summary' } });
  fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
  await screen.findByText('The release is Friday.');
  expect(mocked).toHaveBeenCalledWith('run_workbench_task', { userId: 'default-user', input: {
    provider_id: 'p1', model_id: 'chat1', prompt: 'Summarize the release notes.', skill_id: 'summary', allow_without_skill: false,
  } });
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
  fireEvent.click(screen.getByLabelText('I explicitly want to run without a skill'));
  fireEvent.click(run);
  await waitFor(() => expect(mocked).toHaveBeenCalledWith('run_workbench_task', expect.objectContaining({ input: expect.objectContaining({ skill_id: null, allow_without_skill: true }) })));
});

it('AC22 reports a rejected skill without sending a fallback or showing stale output', async () => {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'run_workbench_task' ? Promise.reject('Selected skill unavailable') : handler(command, args));
  show();
  await configure();
  fireEvent.change(screen.getByLabelText('Skill for this task'), { target: { value: 'summary' } });
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
  fireEvent.change(screen.getByLabelText('Skill for this task'), { target: { value: 'summary' } });
  const button = screen.getByRole('button', { name: 'Run with selected skill' });
  fireEvent.click(button);
  fireEvent.click(button);
  expect(screen.getByRole('button', { name: 'Running task...' }).hasAttribute('disabled')).toBe(true);
  expect(mocked.mock.calls.filter(([name]) => name === 'run_workbench_task')).toHaveLength(1);
  resolve({ record, content: 'Finished once.' });
  await screen.findByText('Finished once.');
});

it('AC22 rejects oversized UTF-8 input before invoking the backend', async () => {
  show();
  await configure();
  fireEvent.change(screen.getByLabelText('Skill for this task'), { target: { value: 'summary' } });
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
  fireEvent.change(screen.getByLabelText('Skill for this task'), { target: { value: 'summary' } });
  fireEvent.click(screen.getByRole('button', { name: 'Run with selected skill' }));
  await screen.findByRole('alert');
  expect(screen.getByText(/Task failed: provider_1001002/)).toBeTruthy();
  expect(screen.queryByText('The release is Friday.')).toBeNull();
});

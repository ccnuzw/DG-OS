import { setLocaleForTesting } from '../src/i18n/runtime';
import { t } from '../src/i18n';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { MemoryRouter } from 'react-router-dom';
import McpManagement from '../src/pages/McpManagement';
import McpConnectionNotice from '../src/components/McpConnectionNotice';
import { useMcpStore } from '../src/stores/mcpStore';
import { useDevelopment } from '../src/stores/developmentStore';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';
import { Modal, ModalHost, ToastViewport, clearToasts } from '@dgos/ui';
import { getToastSnapshot } from '../src/ui/feedback/toastStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);
// jsdom does not implement the native dialog methods used by the desktop UI.
if (!('showModal' in HTMLDialogElement.prototype)) {
  Object.defineProperty(HTMLDialogElement.prototype, 'showModal', {
    configurable: true,
    value(this: HTMLDialogElement) { this.open = true; },
  });
}
if (!('close' in HTMLDialogElement.prototype)) {
  Object.defineProperty(HTMLDialogElement.prototype, 'close', {
    configurable: true,
    value(this: HTMLDialogElement) { this.open = false; },
  });
}
const initial = useMcpStore.getState();
const servers = [
  { id: 'a', name: 'Alpha', transport: 'stdio', enabled: true, status: 'ready' },
  { id: 'b', name: 'Beta', transport: 'http', enabled: false, status: 'configured', credential_ref: 'opaque-ref' },
];
beforeEach(() => {
  setLocaleForTesting('en-US');
  useMcpStore.setState(initial, true);
  vi.spyOn(HTMLDialogElement.prototype, 'showModal').mockImplementation(function (this: HTMLDialogElement) { this.open = true; });
  vi.spyOn(HTMLDialogElement.prototype, 'close').mockImplementation(function (this: HTMLDialogElement) { this.open = false; });
  mocked.mockImplementation(async (command, args) => {
    const id = (args as { serverId?: string } | undefined)?.serverId;
    if (command === 'list_mcp_servers') return servers;
    if (command === 'list_mcp_tools' || command === 'refresh_mcp_tools') return [{
      server_id: id, tool_name: `${id}-tool`, description: `${id} capability`,
      input_schema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] },
    }];
    if (command === 'list_mcp_usage_logs') return [];
    if (command === 'list_mcp_connection_logs') return [{ id: 1, server_id: id, event: `${id}-connected`, status: 'ready', created_at: 1 }];
    if (command === 'create_mcp_server') return { ...servers[0], name: 'New Server', id: 'new', status: 'configured', enabled: false };
    if (command === 'get_mcp_server_config') return {
      name: id === 'a' ? 'Alpha' : 'Beta', transport: id === 'a' ? 'stdio' : 'http',
      stdio: { executable: '/usr/local/bin/server', args: ['--root', '/My Documents'] },
      http: { url: 'https://example.com/mcp', credential_ref: 'opaque-ref' }, enabled: true, auto_connect: false,
    };
    if (command === 'update_mcp_server') return { ...servers[0], id };
    if (command === 'set_mcp_bearer_token') return { ...servers[1], credential_ref: 'new-opaque-ref' };
    if (command === 'clear_mcp_bearer_token') return { ...servers[1], credential_ref: null };
    if (command === 'delete_mcp_server') return undefined;
    if (command === 'confirm_mcp_tool') return 'one-use-consent';
    if (command === 'cancel_mcp_tool') return true;
    if (command === 'call_mcp_tool') return { server_id: 'a', tool_name: 'a-tool', content: [{ type: 'text', text: 'echo result' }], structured_content: { text: 'structured echo result' }, is_error: false, duration_ms: 10, usage_log_id: 1, audit_warning: false };
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(() => { cleanup(); clearToasts(); Modal.closeAll(); vi.restoreAllMocks(); vi.useRealTimers(); });
async function select(name = 'Alpha') {
  fireEvent.click(await screen.findByRole('button', { name: `Select ${name}` }));
  await screen.findByText(name === 'Alpha' ? 'a capability' : 'b capability');
}
// The Transport control is a Radix Select: open it from the keyboard and pick the option by its visible label.
async function chooseTransport(label: string) {
  const trigger = screen.getByRole('combobox', { name: 'Transport' });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  fireEvent.click(within(await screen.findByRole('listbox')).getByRole('option', { name: label }));
}

it('persists explicit remote plaintext consent and resets it whenever the endpoint changes', async () => {
  render(<McpManagement />);
  await screen.findByText('Alpha');
  fireEvent.click(screen.getByRole('button', { name: 'Add MCP server' }));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Remote' } });
  await chooseTransport('HTTP endpoint');
  fireEvent.change(screen.getByLabelText(/^Server URL/), { target: { value: 'http://example.invalid/mcp' } });
  const consent = () => screen.getByRole('checkbox', { name: /I accept remote plaintext HTTP risks/ });
  expect(consent().getAttribute('aria-checked')).toBe('false');
  fireEvent.click(consent());
  expect(consent().getAttribute('aria-checked')).toBe('true');
  fireEvent.change(screen.getByLabelText(/^Server URL/), { target: { value: 'http://other.invalid/mcp' } });
  expect(consent().getAttribute('aria-checked')).toBe('false');
  fireEvent.click(consent());
  fireEvent.click(screen.getByRole('button', { name: 'Save server' }));
  await waitFor(() => expect(mocked).toHaveBeenCalledWith('create_mcp_server', { request: {
    name: 'Remote', transport: 'http', http: { url: 'http://other.invalid/mcp', allow_insecure_http: true },
  } }));
  expect(mocked.mock.calls.some(([name]) => name === 'connect_mcp_server')).toBe(false);
});

it('shows recovery exhaustion outside management and clears the alert on recovery or disable', async () => {
  let current = [{ ...servers[0], status: 'error', reconnect_attempts: 3, reconnect_exhausted: true }];
  mocked.mockResolvedValue(current);
  render(<MemoryRouter initialEntries={['/workbench']} future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><McpConnectionNotice /></MemoryRouter>);
  const alert = await screen.findByRole('alert', { name: 'MCP connection recovery stopped' });
  expect(within(alert).getByText(/Alpha cannot connect/)).toBeTruthy();
  expect(within(alert).getByRole('link', { name: 'Manage connections' }).getAttribute('href')).toBe('/mcp');
  current = [{ ...current[0], status: 'ready', reconnect_attempts: 0, reconnect_exhausted: false }];
  mocked.mockResolvedValue(current);
  await act(() => useMcpStore.getState().fetch());
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  current = [{ ...current[0], enabled: false, status: 'error', reconnect_exhausted: true }];
  mocked.mockResolvedValue(current);
  await act(() => useMcpStore.getState().fetch());
  await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  expect(mocked.mock.calls.every(([name]) => name === 'list_mcp_servers')).toBe(true);
});
async function inspector() {
  await select();
  const button = screen.getByRole('button', { name: 'Inspect / call' }); button.focus(); fireEvent.click(button);
  return screen.findByRole('dialog');
}

it('loads only selected server tools and supports keyboard-operable selection', async () => {
  render(<McpManagement />);
  await select();
  expect(screen.getByRole('button', { name: 'Select Alpha' }).getAttribute('aria-pressed')).toBe('true');
  await screen.findByText('a-connected');
  await select('Beta');
  expect(screen.queryByText('a capability')).toBeNull();
  await screen.findByText('b-connected');
  expect(screen.queryByText('a-connected')).toBeNull();
  expect(screen.getByRole('switch', { name: 'Enable Beta' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Connect', exact: true }).hasAttribute('disabled')).toBe(true);
});

it('preserves spaces in argv and does not connect a saved server automatically', async () => {
  render(<McpManagement />);
  await screen.findByText('Alpha');
  fireEvent.click(screen.getByRole('button', { name: 'Add MCP server' }));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Local' } });
  fireEvent.change(screen.getByLabelText('Executable'), { target: { value: '/usr/local/bin/server' } });
  fireEvent.change(screen.getByLabelText(/^Arguments/), { target: { value: '["--root", "/Users/me/My Documents"]' } });
  fireEvent.click(screen.getByRole('button', { name: 'Save server' }));
  await waitFor(() => expect(mocked).toHaveBeenCalledWith('create_mcp_server', { request: {
    name: 'Local', transport: 'stdio', stdio: { executable: '/usr/local/bin/server', args: ['--root', '/Users/me/My Documents'] },
  } }));
  expect(mocked.mock.calls.some(([name]) => name === 'connect_mcp_server')).toBe(false);
  expect(mocked.mock.calls.some(([, args]) => args && 'userId' in args)).toBe(false);
});

it('allows incomplete argv editing but rejects malformed JSON without saving', async () => {
  render(<McpManagement />);
  await screen.findByText('Alpha');
  fireEvent.click(screen.getByRole('button', { name: 'Add MCP server' }));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Local' } });
  fireEvent.change(screen.getByLabelText('Executable'), { target: { value: '/usr/local/bin/server' } });
  fireEvent.change(screen.getByLabelText(/^Arguments/), { target: { value: '["--root",' } });
  expect((screen.getByLabelText(/^Arguments/) as HTMLInputElement).value).toBe('["--root",');
  fireEvent.click(screen.getByRole('button', { name: 'Save server' }));
  expect(mocked.mock.calls.some(([name]) => name === 'create_mcp_server')).toBe(false);
});

it('edits the clicked server rather than the previously selected server and persists auto-connect', async () => {
  render(<McpManagement />);
  await select('Beta');
  fireEvent.click(screen.getByRole('button', { name: 'Edit Alpha' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  expect((screen.getByLabelText(/^Arguments/) as HTMLInputElement).value).toBe('["--root","/My Documents"]');
  fireEvent.click(screen.getByLabelText('Connect automatically on app startup'));
  fireEvent.click(screen.getByRole('button', { name: 'Save changes' }));
  await waitFor(() => expect(mocked).toHaveBeenCalledWith('update_mcp_server', { serverId: 'a', request: {
    name: 'Alpha', transport: 'stdio', stdio: { executable: '/usr/local/bin/server', args: ['--root', '/My Documents'] }, enabled: true, auto_connect: true,
  } }));
  await waitFor(() => expect(screen.queryByRole('form', { name: 'Server configuration' })).toBeNull());
  expect(screen.getByText('Not checked yet.')).toBeTruthy();
  expect(screen.queryByText('Connect the server to discover tools.')).toBeNull();
  expect(screen.queryByText('a capability')).toBeNull();
  fireEvent.click(within(screen.getByText('Not checked yet.').closest('[role="status"]') as HTMLElement).getByRole('button', { name: 'Retry' }));
  await screen.findByText('a capability');
  expect(screen.queryByText('Not checked yet.')).toBeNull();
});

it('requires explicit review and confirmation and sends exactly the reviewed parameters once', async () => {
  let resolve!: (value: unknown) => void;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'confirm_mcp_tool' ? new Promise(r => { resolve = r; }) : handler(command, args));
  render(<McpManagement />);
  const dialog = await inspector();
  expect(within(dialog).getByText('Input schema')).toBeTruthy();
  expect(dialog.contains(document.activeElement)).toBe(true);
  fireEvent.change(within(dialog).getByLabelText('Arguments (JSON)'), { target: { value: '{"text":"reviewed"}' } });
  fireEvent.click(within(dialog).getByRole('button', { name: 'Review tool call', exact: true }));
  expect(mocked.mock.calls.some(([name]) => name === 'call_mcp_tool' || name === 'confirm_mcp_tool')).toBe(false);
  const confirm = within(dialog).getByRole('button', { name: 'Confirm and call tool' });
  fireEvent.click(confirm); fireEvent.click(confirm);
  expect(mocked.mock.calls.filter(([name]) => name === 'confirm_mcp_tool')).toHaveLength(1);
  resolve('one-use-consent');
  await within(dialog).findByText('Tool completed');
  expect(within(dialog).getByText('Structured result')).toBeTruthy();
  expect(within(dialog).getByText(/structured echo result/)).toBeTruthy();
  const reviewed = (mocked.mock.calls.find(([name]) => name === 'confirm_mcp_tool')![1] as { request: object }).request;
  expect(reviewed).toEqual({ server_id: 'a', tool_name: 'a-tool', arguments: { text: 'reviewed' }, timeout_ms: 30000, call_id: expect.any(String) });
  expect(mocked).toHaveBeenCalledWith('call_mcp_tool', { request: { ...reviewed, confirmation_token: 'one-use-consent' } });
  expect(mocked.mock.calls.filter(([name]) => name === 'call_mcp_tool')).toHaveLength(1);
});

it('closing an inspector without consent does not call tools and restores focus', async () => {
  render(<McpManagement />); const dialog = await inspector();
  fireEvent.click(dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!);
  expect(screen.queryByRole('dialog')).toBeNull();
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Inspect / call' })));
  expect(mocked.mock.calls.some(([name]) => name === 'confirm_mcp_tool' || name === 'call_mcp_tool')).toBe(false);
});

it('shows reported tool errors and audit warnings without retrying a completed side effect', async () => {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'call_mcp_tool' ? Promise.resolve({ is_error: true, content: 'external failure', audit_warning: true, duration_ms: 12, usage_log_id: 7 }) : handler(command, args));
  render(<McpManagement />); const dialog = await inspector();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Review tool call', exact: true }));
  fireEvent.click(within(dialog).getByRole('button', { name: 'Confirm and call tool' }));
  await within(dialog).findByText('Tool reported an error');
  expect(within(dialog).getByRole('alert').textContent).toContain('Do not retry automatically');
  expect(mocked.mock.calls.filter(([name]) => name === 'call_mcp_tool')).toHaveLength(1);
});

it('keeps credentials separate from configs and clears the password field after submission', async () => {
  render(<McpManagement />); await select('Beta');
  const password = screen.getByLabelText('Bearer token') as HTMLInputElement;
  fireEvent.change(password, { target: { value: 'synthetic-ui-token' } });
  fireEvent.click(screen.getByRole('button', { name: 'Store token' }));
  await screen.findByText(/Token stored in the OS keychain/);
  expect(password.value).toBe('');
  expect(mocked).toHaveBeenCalledWith('set_mcp_bearer_token', { serverId: 'b', token: 'synthetic-ui-token' });
  expect(mocked.mock.calls.some(([name]) => name === 'create_mcp_server' || name === 'update_mcp_server')).toBe(false);
  expect(JSON.stringify(useMcpStore.getState())).not.toContain('synthetic-ui-token');
});

it('refreshes tools through discovery rather than only reading the previous snapshot', async () => {
  render(<McpManagement />); await select();
  fireEvent.click(screen.getByRole('button', { name: 'Refresh tools' }));
  await waitFor(() => expect(mocked).toHaveBeenCalledWith('refresh_mcp_tools', { serverId: 'a' }));
});

it('cancels while confirmation is pending without dispatching the tool', async () => {
  let confirm!: (value: string) => void;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'confirm_mcp_tool' ? new Promise(resolve => { confirm = resolve; }) : handler(command, args));
  render(<McpManagement />); const dialog = await inspector();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Review tool call', exact: true }));
  fireEvent.click(within(dialog).getByRole('button', { name: 'Confirm and call tool' }));
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel call' }));
  confirm('unused-consent');
  await within(dialog).findByText('Cancelled before tool dispatch. No tool request was sent.');
  expect(mocked.mock.calls.some(([name]) => name === 'call_mcp_tool' || name === 'cancel_mcp_tool')).toBe(false);
});

it('cancels the exact in-flight call without waiting for the per-server call lock', async () => {
  let finish!: (value: unknown) => void;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'call_mcp_tool' ? new Promise(resolve => { finish = resolve; }) : handler(command, args));
  render(<McpManagement />); const dialog = await inspector();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Review tool call', exact: true }));
  fireEvent.click(within(dialog).getByRole('button', { name: 'Confirm and call tool' }));
  await waitFor(() => expect(mocked.mock.calls.some(([name]) => name === 'call_mcp_tool')).toBe(true));
  const { request } = mocked.mock.calls.find(([name]) => name === 'call_mcp_tool')![1] as { request: { call_id: string } };
  expect(useMcpStore.getState().pending).toContain('a');
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel call' }));
  await waitFor(() => expect(mocked).toHaveBeenCalledWith('cancel_mcp_tool', { serverId: 'a', callId: request.call_id }));
  await within(dialog).findByText(/Cancellation requested. External effects/);
  expect(useMcpStore.getState().pending).toContain('a');
  finish({ outcome: 'cancelled', call_id: request.call_id, is_error: true, content: {}, duration_ms: 10, usage_log_id: 1, audit_warning: false });
  await within(dialog).findByText('Stopped waiting for tool');
  expect(within(dialog).getByRole('alert').textContent).toContain('does not roll back');
  expect(useMcpStore.getState().pending).not.toContain('a');
  expect(mocked.mock.calls.filter(([name]) => name === 'call_mcp_tool')).toHaveLength(1);
});

it('keeps a call running when cancellation fails and does not pretend that it stopped', async () => {
  let finish!: (value: unknown) => void;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => {
    if (command === 'call_mcp_tool') return new Promise(resolve => { finish = resolve; });
    if (command === 'cancel_mcp_tool') return Promise.reject(new Error('Native cancellation unavailable'));
    return handler(command, args);
  });
  render(<McpManagement />); const dialog = await inspector();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Review tool call', exact: true }));
  fireEvent.click(within(dialog).getByRole('button', { name: 'Confirm and call tool' }));
  await waitFor(() => expect(mocked.mock.calls.some(([name]) => name === 'call_mcp_tool')).toBe(true));
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel call' }));
  await within(dialog).findByText(/Could not request cancellation: Native cancellation unavailable/);
  expect(within(dialog).getByRole('button', { name: 'Cancel call' }).hasAttribute('disabled')).toBe(false);
  expect(useMcpStore.getState().pending).toContain('a');
  finish({ outcome: 'success', is_error: false, content: [], duration_ms: 10, usage_log_id: 1, audit_warning: false });
  await within(dialog).findByText('Tool completed');
  expect(mocked.mock.calls.filter(([name]) => name === 'call_mcp_tool')).toHaveLength(1);
});

function renderWithFeedback() {
  return render(<MemoryRouter  future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><McpManagement /><ToastViewport /><ModalHost /></MemoryRouter>);
}
function failing(command: string, error: unknown) {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((name, args) => name === command ? Promise.reject(error) : handler(name, args));
}

it('routes a failed server action through the unified error toast and keeps the inline store error', async () => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  failing('disconnect_mcp_server', { code: 1000000 });
  renderWithFeedback();
  fireEvent.click(await screen.findByRole('button', { name: 'Disconnect' }));
  expect(await screen.findByText('Operation failed')).toBeTruthy();
  expect(screen.getByText('MCP operation failed. Please retry.')).toBeTruthy();
  expect(useMcpStore.getState().error).toBe('MCP operation failed. Please retry.');
});

it('opens the blocking error dialog for a severe host error from deleting a server', async () => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  failing('delete_mcp_server', { code: 1001015 });
  renderWithFeedback();
  await select();
  fireEvent.click(screen.getAllByRole('button', { name: 'Delete' })[0]);
  fireEvent.click(within(await screen.findByRole('dialog', { name: 'Delete MCP server?' })).getByRole('button', { name: 'Delete' }));
  const dialog = await screen.findByRole('dialog', { name: 'OS credential store unavailable' });
  expect(dialog).toBeTruthy();
  expect(screen.queryByRole('dialog', { name: 'Delete MCP server?' })).toBeNull();
  expect(useMcpStore.getState().servers.map(server => server.id)).toContain('a');
});

function deleteCalls() { return mocked.mock.calls.filter(([name]) => name === 'delete_mcp_server'); }
function clearCalls() { return mocked.mock.calls.filter(([name]) => name === 'clear_mcp_bearer_token'); }

it('deletes a server only after the dialog is confirmed, once however fast it is clicked', async () => {
  let finish!: () => void;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'delete_mcp_server' ? new Promise(resolve => { finish = () => resolve(undefined); }) : handler(command, args));
  renderWithFeedback();
  await select();

  fireEvent.click(screen.getAllByRole('button', { name: 'Delete' })[0]);
  let dialog = await screen.findByRole('dialog', { name: 'Delete MCP server?' });
  expect(within(dialog).getByText('Delete Alpha? This disconnects its server.')).toBeTruthy();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(deleteCalls()).toHaveLength(0);

  fireEvent.click(screen.getAllByRole('button', { name: 'Delete' })[0]);
  dialog = await screen.findByRole('dialog', { name: 'Delete MCP server?' });
  const confirmButton = within(dialog).getByRole('button', { name: 'Delete' });
  fireEvent.click(confirmButton); fireEvent.click(confirmButton);
  await waitFor(() => expect(deleteCalls()).toHaveLength(1));
  expect(deleteCalls()[0][1]).toEqual({ serverId: 'a' });
  expect(screen.getByRole('dialog', { name: 'Delete MCP server?' })).toBeTruthy(); // stays open while the host call runs
  expect(within(dialog).getByRole('button', { name: 'Cancel' }).hasAttribute('disabled')).toBe(true);
  await act(async () => { finish(); });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(screen.queryByText('Alpha')).toBeNull();
  expect(deleteCalls()).toHaveLength(1);
});

it('opens one delete dialog for repeated clicks and Esc cancels without deleting', async () => {
  renderWithFeedback();
  await select();
  const trigger = screen.getAllByRole('button', { name: 'Delete' })[0];
  fireEvent.click(trigger); fireEvent.click(trigger);
  const dialog = await screen.findByRole('dialog', { name: 'Delete MCP server?' });
  expect(screen.getAllByRole('dialog')).toHaveLength(1);
  fireEvent.keyDown(dialog, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(deleteCalls()).toHaveLength(0);
  // The cancelled dialog released the guard: the same button opens a new one.
  fireEvent.click(screen.getAllByRole('button', { name: 'Delete' })[0]);
  expect(await screen.findByRole('dialog', { name: 'Delete MCP server?' })).toBeTruthy();
});

it('leaves editing alone when another server is deleted and ends it when the edited server is deleted', async () => {
  renderWithFeedback();
  await screen.findByText('Alpha');
  fireEvent.click(screen.getByRole('button', { name: 'Edit Alpha' }));
  await screen.findByText('Edit Alpha');
  await act(async () => { await useMcpStore.getState().remove('b'); });
  await waitFor(() => expect(screen.queryByText('Beta')).toBeNull());
  expect(screen.getByText('Edit Alpha')).toBeTruthy();
  await act(async () => { await useMcpStore.getState().remove('a'); });
  await waitFor(() => expect(screen.queryByText('Edit Alpha')).toBeNull());
  expect(screen.queryByRole('form', { name: 'Server configuration' })).toBeNull();
});

it('removes a bearer token only after the dialog is confirmed', async () => {
  let finish!: (value: unknown) => void;
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'clear_mcp_bearer_token' ? new Promise(resolve => { finish = resolve; }) : handler(command, args));
  renderWithFeedback();
  await select('Beta');
  const trigger = screen.getByRole('button', { name: 'Remove token' });
  fireEvent.click(trigger); fireEvent.click(trigger);
  let dialog = await screen.findByRole('dialog', { name: 'Remove token?' });
  expect(screen.getAllByRole('dialog')).toHaveLength(1);
  expect(within(dialog).getByText('Remove this server token and disconnect?')).toBeTruthy();
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(clearCalls()).toHaveLength(0);

  fireEvent.click(screen.getByRole('button', { name: 'Remove token' }));
  dialog = await screen.findByRole('dialog', { name: 'Remove token?' });
  const confirmButton = within(dialog).getByRole('button', { name: 'Remove' });
  fireEvent.click(confirmButton); fireEvent.click(confirmButton);
  await waitFor(() => expect(clearCalls()).toHaveLength(1));
  expect(clearCalls()[0][1]).toEqual({ serverId: 'b' });
  expect(screen.getByRole('dialog', { name: 'Remove token?' })).toBeTruthy();
  await act(async () => { finish({ ...servers[1], credential_ref: null }); });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(clearCalls()).toHaveLength(1);
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Remove token' })).toBeNull());
});

it('reports a failed token removal once and closes the dialog', async () => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  failing('clear_mcp_bearer_token', { code: 1000000 });
  renderWithFeedback();
  await select('Beta');
  fireEvent.click(screen.getByRole('button', { name: 'Remove token' }));
  fireEvent.click(within(await screen.findByRole('dialog', { name: 'Remove token?' })).getByRole('button', { name: 'Remove' }));
  expect(await screen.findAllByText('Operation failed')).toHaveLength(1);
  expect(screen.queryByRole('dialog', { name: 'Remove token?' })).toBeNull();
});

it('reports a rejected tool confirmation once and keeps the inline dialog error', async () => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  failing('confirm_mcp_tool', { code: 1000000 });
  renderWithFeedback();
  fireEvent.click(await screen.findByRole('button', { name: 'Select Alpha' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Inspect / call' }));
  const dialog = await screen.findByRole('dialog', { name: 'a-tool' });
  fireEvent.click(within(dialog).getByRole('button', { name: 'Review tool call', exact: true }));
  fireEvent.click(within(dialog).getByRole('button', { name: 'Confirm and call tool' }));
  await within(dialog).findByText('MCP operation failed. Please retry.');
  expect(await screen.findAllByText('Operation failed')).toHaveLength(1);
  expect(mocked.mock.calls.some(([name]) => name === 'call_mcp_tool')).toBe(false);
});

it('does not raise a host error toast for argument validation that never reached the host', async () => {
  vi.spyOn(console, 'error').mockImplementation(() => {});
  renderWithFeedback();
  await screen.findByText('Alpha');
  fireEvent.click(screen.getByRole('button', { name: 'Add MCP server' }));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Local' } });
  fireEvent.change(screen.getByLabelText('Executable'), { target: { value: '/usr/local/bin/server' } });
  fireEvent.change(screen.getByLabelText(/^Arguments/), { target: { value: '["--root",' } });
  fireEvent.click(screen.getByRole('button', { name: 'Save server' }));
  expect(screen.getByLabelText(/^Arguments/).getAttribute('aria-invalid')).toBe('true');
  expect(mocked.mock.calls.some(([name]) => name === 'create_mcp_server')).toBe(false);
  expect(screen.queryByText('Operation failed')).toBeNull();
  expect(console.error).not.toHaveBeenCalled();
});

it('toggles a server through its switch and reflects the host result', async () => {
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'toggle_mcp_server'
    ? Promise.resolve({ ...servers[0], enabled: (args as { enabled: boolean }).enabled })
    : handler(command, args));
  render(<McpManagement />);
  const toggle = await screen.findByRole('switch', { name: 'Enable Alpha' });
  expect(toggle.getAttribute('aria-checked')).toBe('true');
  fireEvent.click(toggle);
  await waitFor(() => expect(mocked).toHaveBeenCalledWith('toggle_mcp_server', { serverId: 'a', enabled: false }));
  await waitFor(() => expect(screen.getByRole('switch', { name: 'Enable Alpha' }).getAttribute('aria-checked')).toBe('false'));
});

// D111: a status poll that keeps failing is shown inline and never as a Toast.
it('D111 keeps repeated status-poll failures inline: no Toast after 30 s of failing polls', async () => {
  const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
  const handler = mocked.getMockImplementation()!;
  let listCalls = 0;
  mocked.mockImplementation((command, args) => {
    if (command !== 'list_mcp_servers') return handler(command, args);
    listCalls += 1;
    return listCalls === 1 ? Promise.resolve(servers) : Promise.reject('MCP status service unreachable');
  });
  vi.useFakeTimers();
  render(<MemoryRouter  future={{ v7_startTransition: true, v7_relativeSplatPath: true }}><McpManagement /><McpConnectionNotice /><ToastViewport /><ModalHost /></MemoryRouter>);
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(screen.getByText('Alpha')).toBeTruthy();
  expect(screen.queryByRole('alert')).toBeNull();

  // Three 10 s polls, each rejected.
  await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
  expect(listCalls).toBeGreaterThanOrEqual(4);
  const alerts = screen.getAllByRole('alert');
  expect(alerts).toHaveLength(1);
  expect(alerts[0].textContent).toContain('MCP status service unreachable');
  expect(getToastSnapshot().visible).toHaveLength(0);
  expect(getToastSnapshot().queued).toHaveLength(0);

  // More failing polls do not queue anything either, and the inline error stays a single block.
  await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
  expect(getToastSnapshot().visible).toHaveLength(0);
  expect(getToastSnapshot().queued).toHaveLength(0);
  expect(screen.getAllByRole('alert')).toHaveLength(1);
  expect(logged).not.toHaveBeenCalledWith('[dgos] error', expect.anything());
});

// D112: URL fields use inputMode="url"; Input keeps its four types.
it('D112 marks the MCP Server URL input inputmode="url" without giving it type="url"', async () => {
  render(<McpManagement />);
  await screen.findByText('Alpha');
  fireEvent.click(screen.getByRole('button', { name: 'Add MCP server' }));
  await chooseTransport('HTTP endpoint');
  const url = screen.getByLabelText(/^Server URL/) as HTMLInputElement;
  expect(url.getAttribute('inputmode')).toBe('url');
  expect(url.type).toBe('text');
});

it('retries a failed configuration read without saving an empty configuration', async () => {
  const first = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'get_mcp_server_config' ? first.promise : handler(command, args));
  render(<McpManagement />);
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
  expect(screen.getByText('Loading configuration…')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Save changes' }).hasAttribute('disabled')).toBe(true);
  await act(async () => { first.reject('configuration unavailable'); await first.promise.catch(() => {}); });
  const alert = screen.getByRole('alert');
  expect(alert.textContent).toContain('configuration unavailable');
  mocked.mockImplementation(handler);
  fireEvent.click(within(alert).getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  expect(screen.getByRole('button', { name: 'Save changes' }).hasAttribute('disabled')).toBe(false);
  expect(screen.queryByText('configuration unavailable')).toBeNull();
  expect(mocked.mock.calls.some(([command]) => command === 'update_mcp_server')).toBe(false);
});

it('preserves the selected server and configuration draft while status refreshes slowly and fails', async () => {
  const handler = mocked.getMockImplementation()!;
  render(<McpManagement />); await select();
  fireEvent.click(screen.getByRole('button', { name: 'Edit Alpha' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Unsaved Alpha' } });
  const name = screen.getByLabelText('Name');
  const refresh = Promise.withResolvers<unknown>();
  mocked.mockImplementation((command, args) => command === 'list_mcp_servers' ? refresh.promise : handler(command, args));
  fireEvent.click(screen.getByRole('button', { name: 'Refresh status' }));
  expect(screen.getByLabelText('Name')).toBe(name);
  expect((name as HTMLInputElement).value).toBe('Unsaved Alpha');
  expect(screen.getByRole('button', { name: 'Select Alpha' }).getAttribute('aria-pressed')).toBe('true');
  await act(async () => { refresh.reject('status refresh unavailable'); await refresh.promise.catch(() => {}); });
  expect(screen.getByRole('alert').textContent).toContain('status refresh unavailable');
  expect(screen.getByLabelText('Name')).toBe(name);
  expect((name as HTMLInputElement).value).toBe('Unsaved Alpha');
});

it('does not replace another configuration draft with a slow read from the previous target', async () => {
  const slow = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'get_mcp_server_config' && args && typeof args === 'object' && 'serverId' in args && args.serverId === 'a' ? slow.promise : handler(command, args));
  render(<McpManagement />);
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
  expect(screen.getByText('Loading configuration…')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  fireEvent.click(screen.getByRole('button', { name: 'Edit Beta' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Beta'));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Beta draft' } });
  await act(async () => { slow.resolve({ name: 'Old Alpha', transport: 'stdio', stdio: { executable: 'old', args: [] } }); await slow.promise; });
  expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Beta draft');
  expect(screen.getByLabelText(/^Server URL/)).toBeTruthy();
});

it('drops a slow configuration failure after the account changes', async () => {
  const slow = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'get_mcp_server_config' ? slow.promise : handler(command, args));
  render(<McpManagement />);
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
  expect(screen.getByText('Loading configuration…')).toBeTruthy();
  mocked.mockImplementation(handler);
  act(() => { useDevelopment.getState().reset(); });
  await screen.findByRole('button', { name: 'Select Beta' });
  await act(async () => { slow.reject('old account configuration unavailable'); await slow.promise.catch(() => {}); });
  expect(screen.queryByRole('form', { name: 'Server configuration' })).toBeNull();
  expect(screen.queryByRole('alert')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Edit Beta' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Beta'));
});

it('keeps invalid argument feedback attached to the draft field and sends no save', async () => {
  render(<McpManagement />);
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  const args = screen.getByLabelText(/^Arguments/);
  fireEvent.change(args, { target: { value: '[1]' } });
  fireEvent.submit(screen.getByRole('form', { name: 'Server configuration' }));
  expect(args.getAttribute('aria-invalid')).toBe('true');
  const descriptions = args.getAttribute('aria-describedby')?.split(' ').map(id => document.getElementById(id)?.textContent ?? '') ?? [];
  expect(descriptions).toContain(t('mcp.jsonArrayError'));
  expect((args as HTMLInputElement).value).toBe('[1]');
  expect(mocked.mock.calls.some(([name]) => name === 'update_mcp_server')).toBe(false);
});

it('retains a rejected configuration draft and synchronously rejects a second save', async () => {
  const save = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'update_mcp_server' ? save.promise : handler(command, args));
  render(<McpManagement />);
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Retained draft' } });
  const form = screen.getByRole('form', { name: 'Server configuration' });
  fireEvent.submit(form); fireEvent.submit(form);
  expect(mocked.mock.calls.filter(([name]) => name === 'update_mcp_server')).toHaveLength(1);
  await act(async () => { save.reject('MCP configuration is invalid: rejected'); await save.promise.catch(() => {}); });
  expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Retained draft');
  expect(screen.getByRole('button', { name: 'Save changes' }).hasAttribute('disabled')).toBe(false);
});

it('does not close or report an old save into a new account configuration editor', async () => {
  const save = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'update_mcp_server' ? save.promise : handler(command, args));
  renderWithFeedback();
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  fireEvent.submit(screen.getByRole('form', { name: 'Server configuration' }));
  act(() => { useDevelopment.getState().reset(); });
  await waitFor(() => expect(screen.queryByRole('form', { name: 'Server configuration' })).toBeNull());
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Beta' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Beta'));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'New account draft' } });
  await act(async () => { save.resolve(servers[0]); await save.promise; });
  expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('New account draft');
  expect(screen.queryByRole('alert')).toBeNull();
  expect(getToastSnapshot().visible).toHaveLength(0);
  expect(getToastSnapshot().queued).toHaveLength(0);
});

it('keeps a confirmed save after a failed tool read and retries only the saved snapshot', async () => {
  const handler = mocked.getMockImplementation()!;
  let failTools = false;
  mocked.mockImplementation((command, args) => {
    if (command === 'update_mcp_server') { failTools = true; return Promise.resolve({ ...servers[0], name: 'Saved Alpha' }); }
    if (command === 'list_mcp_tools' && failTools) return Promise.reject('saved tools unavailable');
    return handler(command, args);
  });
  render(<McpManagement />); await select();
  fireEvent.click(screen.getByRole('button', { name: 'Edit Alpha' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Saved Alpha' } });
  fireEvent.submit(screen.getByRole('form', { name: 'Server configuration' }));
  await waitFor(() => expect(screen.queryByRole('form', { name: 'Server configuration' })).toBeNull());
  expect(screen.getByText(t('mcp.saved'))).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Select Saved Alpha' })).toBeTruthy();
  fireEvent.click(within(screen.getByText('Not checked yet.').closest('[role="status"]') as HTMLElement).getByRole('button', { name: 'Retry' }));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain('saved tools unavailable');
  expect(screen.getByText(t('mcp.saved'))).toBeTruthy();
  failTools = false;
  fireEvent.click(within(alert).getByRole('button', { name: 'Retry' }));
  await screen.findByText('a capability');
  expect(screen.queryByRole('alert')).toBeNull();
  expect(mocked.mock.calls.filter(([command]) => command === 'update_mcp_server')).toHaveLength(1);
  expect(mocked.mock.calls.some(([command]) => command === 'refresh_mcp_tools' || command === 'connect_mcp_server')).toBe(false);
});

it('shares one discard decision between repeated draft cancellation and the surface close guard', async () => {
  const controller = createWorkspaceController();
  render(<SurfaceScope id="mcp" active controller={controller}><McpManagement /><ModalHost /></SurfaceScope>);
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  expect(controller.getSurfaceStatuses().mcp.dirty).toBe(false);
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Retained Alpha' } });
  expect(controller.getSurfaceStatuses().mcp.dirty).toBe(true);
  const cancel = screen.getByRole('button', { name: 'Cancel' });
  fireEvent.click(cancel); fireEvent.click(cancel);
  let leaving!: Promise<boolean>;
  act(() => { leaving = controller.canLeave('mcp'); });
  const dialog = await screen.findByRole('dialog', { name: t('shell.unsavedTitle') });
  expect(screen.getAllByRole('dialog')).toHaveLength(1);
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
  await act(async () => { expect(await leaving).toBe(false); });
  expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Retained Alpha');
  expect(controller.getSurfaceStatuses().mcp.dirty).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  fireEvent.click(within(await screen.findByRole('dialog', { name: t('shell.unsavedTitle') })).getByRole('button', { name: t('shell.discard') }));
  await waitFor(() => expect(screen.queryByRole('form', { name: 'Server configuration' })).toBeNull());
  expect(controller.getSurfaceStatuses().mcp.dirty).toBe(false);
  expect(await controller.canLeave('mcp')).toBe(true);
  expect(mocked.mock.calls.some(([command]) => command === 'update_mcp_server')).toBe(false);
});

it('vetoes surface close while configuration saving is in flight', async () => {
  const save = Promise.withResolvers<unknown>();
  const handler = mocked.getMockImplementation()!;
  mocked.mockImplementation((command, args) => command === 'update_mcp_server' ? save.promise : handler(command, args));
  const controller = createWorkspaceController();
  render(<SurfaceScope id="mcp" active controller={controller}><McpManagement /></SurfaceScope>);
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  fireEvent.submit(screen.getByRole('form', { name: 'Server configuration' }));
  expect(await controller.canLeave('mcp')).toBe(false);
  expect(screen.getByRole('button', { name: 'Cancel' }).hasAttribute('disabled')).toBe(true);
  expect(screen.queryByRole('dialog')).toBeNull();
  await act(async () => { save.resolve(servers[0]); await save.promise; });
  await waitFor(() => expect(screen.queryByRole('form', { name: 'Server configuration' })).toBeNull());
  expect(await controller.canLeave('mcp')).toBe(true);
});

it('closes the old discard dialog on account change and leaves the new configuration draft accessible', async () => {
  renderWithFeedback();
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Old draft' } });
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  await screen.findByRole('dialog', { name: t('shell.unsavedTitle') });
  act(() => { useDevelopment.getState().reset(); });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  fireEvent.click(await screen.findByRole('button', { name: 'Edit Beta' }));
  await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Beta'));
  fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'New account draft' } });
  expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('New account draft');
  expect(mocked.mock.calls.some(([command]) => command === 'update_mcp_server')).toBe(false);
});

it('retains an editing draft through narrow list navigation and width changes without retargeting the save', async () => {
  let resize!: () => void;
  let width = 1200;
  let frame: FrameRequestCallback | undefined;
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (this: HTMLElement) {
    return this.classList.contains('mcp-content') ? width : 0;
  });
  vi.stubGlobal('ResizeObserver', class {
    constructor(private callback: () => void) { resize = callback; }
    observe() {}
    unobserve() {}
    disconnect() {}
  });
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frame = callback; return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => { frame = undefined; });
  const flush = () => act(() => { const callback = frame; frame = undefined; callback?.(0); });
  try {
    renderWithFeedback();
    fireEvent.click(await screen.findByRole('button', { name: 'Edit Alpha' }));
    await waitFor(() => expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Alpha'));
    const input = screen.getByLabelText('Name');
    fireEvent.change(input, { target: { value: 'Alpha draft' } });
    act(() => { width = 500; resize(); });
    flush();
    expect(screen.getByLabelText('Name')).toBe(input);
    fireEvent.click(screen.getByRole('button', { name: 'Your servers · Alpha' }));
    fireEvent.change(screen.getByLabelText('Search servers'), { target: { value: 'Beta' } });
    fireEvent.click(screen.getByRole('button', { name: 'Select Beta' }));
    expect(screen.getByLabelText('Name')).toBe(input);
    expect((input as HTMLInputElement).value).toBe('Alpha draft');
    act(() => { width = 1100; resize(); });
    flush();
    expect((screen.getByLabelText('Search servers') as HTMLInputElement).value).toBe('Beta');
    expect(screen.getByRole('button', { name: 'Edit Beta' }).hasAttribute('disabled')).toBe(true);
    fireEvent.submit(screen.getByRole('form', { name: 'Server configuration' }));
    await waitFor(() => expect(mocked).toHaveBeenCalledWith('update_mcp_server', expect.objectContaining({ serverId: 'a', request: expect.objectContaining({ name: 'Alpha draft' }) })));
  } finally { vi.unstubAllGlobals(); }
});

it('remeasures MCP layout when UI typography changes without a width change', async () => {
  let measureWidth = 950;
  let frame: FrameRequestCallback | undefined;
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (this: HTMLElement) {
    return this.classList.contains('mcp-content') ? measureWidth : 0;
  });
  vi.stubGlobal('ResizeObserver', class {
    constructor(_callback: () => void) {}
    observe() {}
    unobserve() {}
    disconnect() {}
  });
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frame = callback; return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => { frame = undefined; });
  const flush = () => act(() => { const callback = frame; frame = undefined; callback?.(0); });
  try {
    renderWithFeedback();
    await screen.findByText('Alpha');
    flush();
    fireEvent.click(screen.getByRole('button', { name: 'Edit Alpha' }));
    const input = await screen.findByLabelText('Name');
    fireEvent.change(input, { target: { value: 'Typography draft' } });
    measureWidth = 950;
    document.documentElement.style.setProperty('--font-size-ui', '16px');
    await act(async () => {});
    flush();
    expect(screen.getByLabelText('Name')).toBe(input);
    expect((input as HTMLInputElement).value).toBe('Typography draft');
    expect(screen.getByRole('button', { name: 'Your servers · Alpha' })).toBeTruthy();
  } finally {
    document.documentElement.removeAttribute('style');
    vi.unstubAllGlobals();
  }
});
